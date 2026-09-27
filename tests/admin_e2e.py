#!/usr/bin/env python3
"""Run against a disposable migrated test server; creates catalog/order records.
LIYU_TEST_ADMIN_USERNAME/PASSWORD specify the test-only bootstrap credentials.
"""
import http.cookiejar
import json
import os
import pathlib
import sys
import subprocess
import urllib.error
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor

base = sys.argv[1] if len(sys.argv) > 1 else 'http://127.0.0.1:18788'
jar = http.cookiejar.CookieJar()
admin = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
plain = urllib.request.build_opener()
csrf = ''
checks = 0

def call(path, method='GET', body=None, status=200, opener=admin, headers=None, protect=True):
    global checks
    h = dict(headers or {})
    if protect:
        h['X-Admin-Request'] = '1'
        if csrf:
            h['X-CSRF-Token'] = csrf
    if isinstance(body, dict):
        body = json.dumps(body).encode()
        h['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=body, headers=h, method=method)
    try:
        response = opener.open(req)
    except urllib.error.HTTPError as e:
        response = e
    data = response.read()
    assert response.status == status, (path, response.status, status, data[:300])
    checks += 1
    return json.loads(data) if 'application/json' in response.headers.get('Content-Type', '') else data

call('/admin')
call('/admin/', status=200)
call('/admin/api/products', status=401)
credentials = {'username': os.environ['LIYU_TEST_ADMIN_USERNAME'], 'password': os.environ['LIYU_TEST_ADMIN_PASSWORD']}
call('/admin/api/login', 'POST', credentials, status=403, protect=False)
call('/admin/api/login', 'POST', {**credentials, 'password': 'wrong'}, status=401)
call('/admin/api/login', 'POST', {'username':'demo@liyu.test', 'password':'123456'}, status=401)
user = call('/api/v1/auth/login', 'POST', {'identifier':'demo@liyu.test','password':'123456'}, opener=plain)
call('/admin/api/products', status=401, opener=plain, headers={'Authorization':'Bearer '+user['token']})
login = call('/admin/api/login', 'POST', credentials)
csrf = login['csrf_token']
assert any(c.name=='liyu_admin_session' and c.path=='/admin' for c in jar)
call('/admin/api/me')
admin_token = next(c.value for c in jar if c.name=='liyu_admin_session')
call('/api/v1/me', status=401, opener=plain, headers={'Authorization':'Bearer '+admin_token})
call('/admin/api/products','POST',{},status=403, protect=False)
call('/admin/api/products','POST',{},status=403,headers={'Origin':'https://evil.example'})
p={'name':'Admin E2E '+str(uuid.uuid4()),'category':'home','price_cents':2345,'physical':True,'brand':'测试','kind':'礼物','spec':'一件','description':'测试商品','tags':['测试'],'stock':2,'is_active':False}
call('/admin/api/products','POST',{**p,'stock':-1},status=400)
pid=call('/admin/api/products','POST',p)['id']
assert pid>32
call(f'/api/v1/catalog/{pid}',status=404)
listing=call('/admin/api/products?q=Admin%20E2E')
assert any(row['id']==pid for row in listing['items'])
p['is_active']=True
call(f'/admin/api/products/{pid}','PUT',p)
assert call(f'/api/v1/catalog/{pid}')['price_cents']==2345
fixture=(pathlib.Path(__file__).resolve().parents[1]/'test-data/products/p23.png').read_bytes()
call(f'/admin/api/products/{pid}/images/card','POST',b'broken',status=422)
call(f'/admin/api/products/{pid}/images/nope','POST',fixture,status=400)
call('/admin/api/products/2147483647/images/card','POST',fixture,status=404)
call(f'/admin/api/products/{pid}/images/card','POST',fixture,headers={'Content-Type':'image/png'})
for variant in ['thumb','card','detail']:
    assert call(f'/api/v1/media/products/{pid}/{variant}').startswith(b'\x89PNG')
oversize = subprocess.run(['curl','-sS','-o','/dev/null','-w','%{http_code}',
    '-H','X-Admin-Request: 1','-H','X-CSRF-Token: '+csrf,
    '-H','Cookie: liyu_admin_session='+admin_token,'--data-binary','@-',
    base+f'/admin/api/products/{pid}/images/card'],input=b'x'*(4*1024*1024+1),capture_output=True)
assert oversize.stdout == b'413', oversize.stdout
checks += 1
headers={'Authorization':'Bearer '+user['token']}
recipient=call('/api/v1/auth/login','POST',{'identifier':'linzhou@liyu.test','password':'123456'},opener=plain)['user']['id']
call('/api/v1/cart','DELETE',opener=plain,headers=headers)
call('/api/v1/cart/items','POST',{'product_id':pid,'recipient_id':recipient},opener=plain,headers=headers)
order=call('/api/v1/orders','POST',opener=plain,headers={**headers,'Idempotency-Key':str(uuid.uuid4())})
p['is_active']=False
call(f'/admin/api/products/{pid}','PUT',p)
call('/api/v1/cart/items','POST',{'product_id':pid,'recipient_id':recipient},status=409,opener=plain,headers=headers)
call(f'/api/v1/orders/{order["id"]}/pay-test','POST',status=409,opener=plain,headers=headers)
p['is_active']=True
call(f'/admin/api/products/{pid}','PUT',p)
call(f'/api/v1/orders/{order["id"]}/pay-test','POST',opener=plain,headers=headers)
call(f'/api/v1/orders/{order["id"]}/pay-test','POST',opener=plain,headers=headers)
assert call(f'/api/v1/catalog/{pid}')['stock']==1
# Multiple cart lines must reserve the aggregate quantity or roll back completely.
for _ in range(2):
    call('/api/v1/cart/items','POST',{'product_id':pid,'recipient_id':recipient},opener=plain,headers=headers)
multiple=call('/api/v1/orders','POST',opener=plain,headers={**headers,'Idempotency-Key':str(uuid.uuid4())})
call(f'/api/v1/orders/{multiple["id"]}/pay-test','POST',status=409,opener=plain,headers=headers)
assert call(f'/api/v1/catalog/{pid}')['stock']==1
# Two pending orders racing for the final unit must yield one winner.
pending=[]
for _ in range(2):
    call('/api/v1/cart/items','POST',{'product_id':pid,'recipient_id':recipient},opener=plain,headers=headers)
    pending.append(call('/api/v1/orders','POST',opener=plain,headers={**headers,'Idempotency-Key':str(uuid.uuid4())})['id'])
def pay(order_id):
    req=urllib.request.Request(base+f'/api/v1/orders/{order_id}/pay-test',method='POST',headers=headers)
    try:
        with urllib.request.urlopen(req) as response: return response.status
    except urllib.error.HTTPError as response: return response.code
with ThreadPoolExecutor(max_workers=2) as workers:
    assert sorted(workers.map(pay,pending))==[200,409]
checks += 2
assert call(f'/api/v1/catalog/{pid}')['stock']==0
call('/api/v1/cart/items','POST',{'product_id':pid,'recipient_id':recipient},status=409,opener=plain,headers=headers)
call('/admin/api/logout','POST',status=204)
call('/admin/api/me',status=401)
print(f'Admin integration: {checks} HTTP checks passed (independent login, CSRF, CRUD, media, stock, logout).')
