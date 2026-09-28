# coding: utf-8
"""Run against an isolated contact_test database and a test-delivery server."""
import json
import os
import subprocess
import struct
import sys
import urllib.error
import urllib.request
import uuid
import zlib

base = sys.argv[1]
db = os.environ['DATABASE_URL']
assert 'contact_test' in db and base.startswith('http://127.0.0.1:')
checks = 0

def call(path, method='GET', body=None, token=None, status=200, **headers):
    global checks
    h = {'Content-Type': 'application/json', **headers}
    if token: h['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request(base + path, method=method, headers=h,
        data=json.dumps(body).encode() if body is not None else None)
    try: response = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error: response = error
    data = response.read()
    assert response.status == status, (path, response.status, status, data[:300])
    checks += 1
    return json.loads(data) if data and 'application/json' in response.headers.get('Content-Type', '') else data

def sql(statement):
    return subprocess.check_output(['psql', db, '-X', '-At', '-v', 'ON_ERROR_STOP=1', '-c', statement], text=True).strip()

def challenge(kind, value, purpose='bind', token=None):
    return call('/api/v1/auth/challenges', 'POST', dict(kind=kind,value=value,purpose=purpose), token)

def register(email):
    c = challenge('email', email, 'register')
    return call('/api/v1/auth/register','POST',dict(identifier=email,password='test-pass-2026',challenge_id=c['challenge_id'],code=c['test_code']))

def bind(token,kind,value):
    c=challenge(kind,value,token=token)
    return call('/api/v1/me/'+kind,'PUT',dict(value=value,challenge_id=c['challenge_id'],code=c['test_code']),token)

suffix=uuid.uuid4().hex[:12]
sender=call('/api/v1/auth/login','POST',dict(identifier='demo@liyu.test',password='123456'))['token']
a='a-'+suffix+'@example.test'; b='b-'+suffix+'@example.test'; c='c-'+suffix+'@example.test'
extra='extra-'+suffix+'@example.test'
# Make the phone unique and valid without depending on the UUID's letters.
number='+1415'+str(int(suffix,16)%100000000).zfill(8)
first=register(a); first_id=first['user']['id']; first_token=first['token']
bind(first_token,'email',extra)
bind(first_token,'phone',number)
profile=call('/api/v1/me/profile',token=first_token)
assert set([a,extra]).issubset(profile['emails']) and number in profile['phones']
assert sql(f"SELECT count(*) FROM contact_identities WHERE user_id={first_id}")=='3'
# The address-book lookup exposes only a verified owner's chosen photo.
lookup=lambda contacts, token=sender, status=200: call('/api/v1/contacts/avatars','POST',
    {'contacts':contacts},token,status)
addresses=[dict(kind='email',value=a),dict(kind='phone',value=number),
    dict(kind='email',value='missing-'+suffix+'@example.test')]
assert lookup(addresses)['avatars']==[]  # The server's default identicon is omitted.
lookup(addresses,token=None,status=401)
def chunk(kind, data):
    return struct.pack('!I',len(data))+kind+data+struct.pack('!I',zlib.crc32(kind+data))
raw=b''.join([b'\x00'+b'\x33\x88\xcc\xff'*16 for _ in range(16)])
png=(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',16,16,8,6,0,0,0))+
    chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
request=urllib.request.Request(base+'/api/v1/me/avatar',method='POST',data=png,
    headers={'Authorization':'Bearer '+first_token,'Content-Type':'image/png'})
uploaded=json.loads(urllib.request.urlopen(request,timeout=10).read())
avatar_url=uploaded['avatar_url']
assert avatar_url.startswith('/api/v1/media/avatars/')
found=lookup(addresses+addresses[:1])['avatars']
assert len(found)==2 and all(row['avatar_url']==avatar_url for row in found)
call('/api/v1/me/avatar','DELETE',token=first_token,status=204)
assert lookup(addresses)['avatars']==[]
contact=call('/api/v1/contacts','POST',dict(label='好友',phones=[number],emails=[a,extra]),sender)
assert sql(f"SELECT bound_user_id FROM sender_contacts WHERE id={contact['id']}")==str(first_id)
assert len(call('/api/v1/contacts',token=sender)[0]['emails'])==2
body=dict(product_id=0,recipient=dict(kind='email',value=a,label='好友'))
assert call('/api/v1/orders/quote','POST',body,sender)['recipient_warning'] is None
# The old owner releases this address. A second account verifies and binds it.
call('/api/v1/me/contact-identities','DELETE',dict(kind='email',value=a),first_token)
assert call('/api/v1/orders/quote','POST',body,sender)['recipient_warning']['code']=='recipient_identity_changed'
sql(f"UPDATE contact_challenges SET created_at=created_at-interval '2 minutes' WHERE kind='email' AND value='{a}'")
second=register(b); second_id=second['user']['id']; second_token=second['token']
bind(second_token,'email',a)
warning=call('/api/v1/orders/quote','POST',body,sender)['recipient_warning']
assert warning['code']=='recipient_identity_changed' and 'current_user_id' not in warning
call('/api/v1/orders','POST',body,sender,409,**{'Idempotency-Key':str(uuid.uuid4())})
assert sql(f"SELECT expired_at IS NULL FROM sender_contacts WHERE id={contact['id']}")=='t'
body['confirm_recipient_change']=True
order=call('/api/v1/orders','POST',body,sender,**{'Idempotency-Key':str(uuid.uuid4())})
assert sql(f"SELECT expired_at IS NULL FROM sender_contacts WHERE id={contact['id']}")=='t'
assert sql(f"SELECT recipient_change_confirmed FROM order_items WHERE order_id={order['id']}")=='t'
stable_order=call('/api/v1/orders','POST',body,sender,**{'Idempotency-Key':str(uuid.uuid4())})
call(f"/api/v1/orders/{stable_order['id']}/pay-test",'POST',{},sender)
assert sql(f"SELECT expired_at IS NOT NULL FROM sender_contacts WHERE id={contact['id']}")=='t'
assert sql(f"SELECT count(*) FROM sender_contacts WHERE owner_id=1 AND bound_user_id={first_id} AND expired_at IS NULL")=='1'
assert sql(f"SELECT count(*) FROM sender_contacts WHERE owner_id=1 AND bound_user_id={second_id} AND expired_at IS NULL")=='1'
# A third verified owner appears between order creation and payment.
call('/api/v1/me/contact-identities','DELETE',dict(kind='email',value=a),second_token)
sql(f"UPDATE contact_challenges SET created_at=created_at-interval '2 minutes' WHERE kind='email' AND value='{a}'")
third=register(c); third_id=third['user']['id']; bind(third['token'],'email',a)
pay=f"/api/v1/orders/{order['id']}/pay-test"
warning=call(pay,'POST',{},sender,409)
assert warning['code']=='recipient_identity_changed' and 'current_user_id' not in warning
assert sql(f"SELECT status FROM orders WHERE id={order['id']}")=='pending'
call(pay,'POST',{},sender,**{'X-Confirm-Recipient-Change':'true'})
assert sql(f"SELECT recipient_id FROM gifts WHERE id=(SELECT gift_id FROM order_items WHERE order_id={order['id']})")==str(third_id)
assert sql(f"SELECT count(*) FROM sender_contacts WHERE owner_id=1 AND bound_user_id={third_id} AND expired_at IS NULL")=='1'
# An unregistered address is anchored when its first owner registers.
pending='pending-'+suffix+'@example.test'
p=call('/api/v1/contacts','POST',dict(label='待注册',emails=[pending]),sender)
assert sql(f"SELECT bound_user_id IS NULL FROM sender_contacts WHERE id={p['id']}")=='t'
new=register(pending)
assert sql(f"SELECT bound_user_id FROM sender_contacts WHERE id={p['id']}")==str(new['user']['id'])
# One address-book entry cannot merge two already verified accounts.
call('/api/v1/contacts','POST',dict(label='混合',emails=[extra,b]),sender,409)
print(f'{checks} contact identity HTTP checks passed')
