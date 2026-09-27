# coding: utf-8
"""Controlled HTTP/real PostgreSQL contract tests; never use a business database.

Start the server with LIYU_TEST_DELIVERY=true, LIYU_PUBLIC_URL=<base>,
LIYU_DELIVERY_WEBHOOK=http://127.0.0.1:18818/send and DATABASE_URL pointing
to a disposable database whose name includes contact_test. No real messages.
"""
import concurrent.futures
import http.cookiejar
import http.server
import json
import os
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

base = sys.argv[1] if len(sys.argv)>1 else 'http://127.0.0.1:18817'
url = os.environ['DATABASE_URL']
assert 'contact_test' in urllib.parse.urlsplit(url).path
assert urllib.parse.urlsplit(base).hostname=='127.0.0.1'
checks=0
received=[]
attempts=[]

class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        data=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        attempts.append(data)
        # Real HTTP provider failure followed by retry, using the same event key.
        status=503 if len(attempts)==1 else 200
        if status==200:received.append(data)
        self.send_response(status);self.end_headers()
    def log_message(self,*args):pass

provider=http.server.HTTPServer(('127.0.0.1',18818),Provider)
threading.Thread(target=provider.serve_forever,daemon=True).start()

def sql(query):
    result=subprocess.run(['psql',url,'-X','-At','-v','ON_ERROR_STOP=1','-c',query],capture_output=True,text=True,check=True)
    return result.stdout.strip()

def call(path,method='GET',body=None,token=None,status=200,key=None):
    global checks
    h={'Content-Type':'application/json'}
    if token:h['Authorization']='Bearer '+token
    if key:h['Idempotency-Key']=key
    request=urllib.request.Request(base+path,headers=h,method=method,data=json.dumps(body).encode() if body is not None else None)
    try:r=urllib.request.urlopen(request,timeout=10)
    except urllib.error.HTTPError as e:r=e
    data=r.read();assert r.status==status,(path,r.status,status,data[:300]);checks+=1
    return json.loads(data) if 'application/json' in r.headers.get('Content-Type','') else data

def challenge(kind,value,purpose='register',token=None):
    return call('/api/v1/auth/challenges','POST',dict(kind=kind,value=value,purpose=purpose),token)

def register(identity):
    c=challenge('email' if '@' in identity else 'phone',identity)
    return call('/api/v1/auth/register','POST',dict(identifier=identity,password='contact-pass-2026',code=c['test_code'],challenge_id=c['challenge_id']))

sql("UPDATE delivery_outbox SET status='cancelled' WHERE status<>'sent'")
sender=call('/api/v1/auth/login','POST',dict(identifier='demo@liyu.test',password='123456'))['token']
suffix=uuid.uuid4().hex[:12]
registered=register('recipient-'+suffix+'@example.test')
recipient=registered['token'];uid=registered['user']['id']
assert sql(f"SELECT count(*) FROM friendships WHERE user_low_id=LEAST(1,{uid}) AND user_high_id=GREATEST(1,{uid})")=='0'

def send(kind,value,label='同名收件人'):
    call('/api/v1/cart','DELETE',token=sender)
    call('/api/v1/cart/items','POST',dict(product_id=0,recipient=dict(kind=kind,value=value,label=label)),sender)
    cart=call('/api/v1/cart',token=sender)
    assert len(cart['items'])==1 and cart['items'][0]['recipient_id'] is None
    key=str(uuid.uuid4());order=call('/api/v1/orders','POST',{},sender,key=key)
    assert call('/api/v1/orders','POST',{},sender,key=key)['id']==order['id']
    call(f"/api/v1/orders/{order['id']}/pay-test",'POST',{},sender)
    call(f"/api/v1/orders/{order['id']}/pay-test",'POST',{},sender)
    detail=call(f"/api/v1/orders/{order['id']}",token=sender)
    return detail['items'][0]['gift_id']

gift=send('email','recipient-'+suffix+'@example.test')
assert sql(f'SELECT recipient_id FROM gifts WHERE id={gift}')==str(uid)
assert sql(f'SELECT count(*) FROM notifications WHERE gift_id={gift}')=='1'
assert sql(f'SELECT count(*) FROM delivery_outbox WHERE gift_id={gift}')=='0'
notifications=call('/api/v1/notifications',token=recipient)
n=next(n for n in notifications if n['gift_id']==gift)
call(f"/api/v1/notifications/{n['id']}/read",'POST',{},sender,status=404)
call(f"/api/v1/notifications/{n['id']}/read",'POST',{},recipient)
call('/api/v1/cart/items','POST',dict(product_id=0,recipient=dict(kind='email',value='recipient-'+suffix+'@example.test')),recipient,status=400)

unknown='waiting-'+suffix+'@example.test'
pending=send('email',unknown)
assert sql(f'SELECT recipient_id IS NULL FROM gifts WHERE id={pending}')=='t'
sent=call('/api/v1/gifts/outbox',token=sender)
assert any(g['id']==pending and g['delivery_status']=='pending_claim' for g in sent)
assert sql(f'SELECT count(*) FROM gifts WHERE id={pending}')=='1'
deadline=time.time()+15
while not attempts and time.time()<deadline:time.sleep(.2)
assert attempts,'worker did not call provider'
sql(f"UPDATE delivery_outbox SET next_attempt_at=now() WHERE gift_id={pending}")
deadline=time.time()+15
while not received and time.time()<deadline:time.sleep(.2)
assert received,'worker did not retry provider'
assert attempts[0]['event_key']==received[0]['event_key']
assert attempts[0]['payload']['type']=='gift_invitation'
assert 'demo' not in json.dumps(received[0]['payload'])
invitation=urllib.parse.urlsplit(received[0]['payload']['url']).path.split('/')[-1]
call('/gift-invitations/'+invitation)
call('/api/v1/gift-invitations/'+invitation+'/claim','POST',{},recipient,status=404)
c=challenge('email',unknown)
call('/api/v1/auth/challenges','POST',dict(kind='email',value=unknown,purpose='register'),status=429)
body=dict(identifier=unknown,password='contact-pass-2026',challenge_id=c['challenge_id'],code='000000')
call('/api/v1/auth/register','POST',body,status=400)
assert sql(f"SELECT attempts FROM contact_challenges WHERE id='{c['challenge_id']}'")=='1'
body['code']=c['test_code']
new=call('/api/v1/auth/register','POST',body)
new_token=new['token'];new_id=new['user']['id']
call('/api/v1/auth/register','POST',body,status=400)
assert sql(f'SELECT recipient_id FROM gifts WHERE id={pending}')==str(new_id)
assert sql(f'SELECT count(*) FROM notifications WHERE gift_id={pending}')=='1'
def claim_once(_):return call('/api/v1/gift-invitations/'+invitation+'/claim','POST',{},new_token)
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
    assert all(r['gift_id']==pending for r in executor.map(claim_once,range(2)))
assert sql(f'SELECT count(*) FROM friendships WHERE user_low_id=LEAST(1,{new_id}) AND user_high_id=GREATEST(1,{new_id})')=='0'

# Existing account verifies another channel and picks up an outstanding phone gift.
phone='+1415'+str(int(uuid.uuid4().hex[:7],16)).zfill(7)[-7:]
phone_gift=send('phone',phone)
c=challenge('phone',phone,'bind',recipient)
call('/api/v1/me/phone','PUT',dict(value=phone,code=c['test_code'],challenge_id=c['challenge_id']),new_token,status=400)
call('/api/v1/me/phone','PUT',dict(value=phone,code=c['test_code'],challenge_id=c['challenge_id']),recipient)
assert sql(f'SELECT recipient_id FROM gifts WHERE id={phone_gift}')==str(uid)
profile=call('/api/v1/me/profile',token=recipient)
assert profile['phone']==phone and profile['phone_verified'] is True and profile['email_verified'] is True

withdrawn=send('email','withdraw-'+suffix+'@example.test')
call(f'/api/v1/gifts/{withdrawn}/withdraw','POST',{},sender)
w=register('withdraw-'+suffix+'@example.test')
assert sql(f'SELECT recipient_id IS NULL FROM gifts WHERE id={withdrawn}')=='t'
expired=send('email','expired-'+suffix+'@example.test')
sql(f"UPDATE gifts SET expires_at=now()-interval '1 second' WHERE id={expired}")
deadline=time.time()+15
while sql(f'SELECT state FROM gifts WHERE id={expired}')!='expired' and time.time()<deadline:time.sleep(.2)
assert sql(f'SELECT state FROM gifts WHERE id={expired}')=='expired'
assert sql(f"SELECT count(*) FROM wallet_ledger WHERE gift_id={expired} AND kind='expired'")=='1'
# Admin report and manual retry must never disclose provider destinations or payloads.
retry_gift=send('email','retry-'+suffix+'@example.test')
sql(f"UPDATE delivery_outbox SET status='failed',attempts=8,next_attempt_at=now()+interval '1 hour' WHERE gift_id={retry_gift}")
job=int(sql(f'SELECT id FROM delivery_outbox WHERE gift_id={retry_gift}'))
admin=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def admin_call(path,body=None,csrf=None,status=200):
    global checks
    h={'Content-Type':'application/json','X-Admin-Request':'1'}
    if csrf:h['X-CSRF-Token']=csrf
    r=urllib.request.Request(base+path,headers=h,data=json.dumps(body).encode() if body is not None else None)
    try:response=admin.open(r,timeout=10)
    except urllib.error.HTTPError as e:response=e
    result=json.load(response);assert response.status==status,(path,response.status,result);checks+=1
    return result
csrf=admin_call('/admin/api/login',{'username':os.environ.get('LIYU_TEST_ADMIN_USERNAME','admin'),'password':os.environ['LIYU_TEST_ADMIN_PASSWORD']})['csrf_token']
rows=admin_call('/admin/api/reports/deliveries?q='+str(job))['items']
assert any(r['id']==job for r in rows)
assert all('payload' not in r and 'destination' not in r for r in rows)
admin_call(f'/admin/api/manage/retry-delivery/{job}',{'reason':'isolated test'},status=403)
retried=admin_call(f'/admin/api/manage/retry-delivery/{job}',{'reason':'isolated test'},csrf)
assert retried['status']=='pending' and retried['attempts']==0
assert 'payload' not in retried and 'destination' not in retried
assert sql(f"SELECT count(*) FROM admin_audit WHERE action='retry-delivery' AND entity_id={job} AND NOT(before_data ? 'destination') AND NOT(after_data ? 'payload')")=='1'
provider.shutdown()
print(f'contact delivery HTTP/DB E2E: {checks} checks passed')
