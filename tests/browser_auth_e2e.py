#!/usr/bin/env python3
"""Run against a disposable migrated server with LIYU_TEST_DELIVERY=true."""
import json, urllib.request, urllib.error, sys, uuid, os, subprocess
from concurrent.futures import ThreadPoolExecutor
base=sys.argv[1] if len(sys.argv)>1 else 'http://127.0.0.1:18788'
checks=0

def call(path, body=None, token='', status=200, method='POST'):
    global checks
    headers={'Content-Type':'application/json'}
    if token: headers['Authorization']='Bearer '+token
    request=urllib.request.Request(base+path,data=None if body is None else json.dumps(body).encode(),headers=headers,method=method)
    try: response=urllib.request.urlopen(request)
    except urllib.error.HTTPError as e: response=e
    data=response.read();assert response.status==status,(path,response.status,status,data[:200]);checks+=1
    return json.loads(data) if data else {}

def start(purpose='login',token=''):
    f=call('/api/v1/browser-authorizations',{'purpose':purpose},token)
    f['key']=f['browser_path'].split('#')[1];f['base']='/api/v1/browser-authorizations/'+f['id'];return f

def login(f,identity='demo@liyu.test'):
    r=call(f['base']+'/login',{'browser_key':f['key'],'identifier':identity,'password':'123456'});assert r['expires_in_seconds']==600;return r
def approve(f,t):return call(f['base']+'/approve',{'browser_key':f['key']},t)
def poll(f,suffix=''):return call(f['base']+'/poll'+suffix,{'poll_key':f['poll_key']})

call('/api/v1/browser-authorizations',{'purpose':'wrong'},status=400)
call('/api/v1/browser-authorizations',{'purpose':'email'},status=401)
f=start();assert poll(f)['state']=='pending'
assert call(f['base']+'/poll',{'poll_key':f['key']})['state']=='invalid'
call(f['base']+'/info',{'browser_key':f['poll_key']},status=410)
call(f['base']+'/login',{'browser_key':'wrong','identifier':'demo@liyu.test','password':'123456'},status=410)
t=login(f)['token'];assert call('/api/v1/me',token=t,method='GET')['id']>0
call(f['base']+'/approve',{'browser_key':'wrong'},t,status=409)
approve(f,t);call('/api/v1/me',token=t,method='GET',status=401)
assert poll(f)['state']=='complete';assert poll(f)['state']=='consumed'
# Atomic one-time claim under concurrent polling.
f=start();t=login(f)['token'];approve(f,t)
with ThreadPoolExecutor(max_workers=2) as executor: results=list(executor.map(lambda _:poll(f),range(2)))
assert sorted(r['state'] for r in results)==['complete','consumed']
app=next(r['token'] for r in results if r['state']=='complete')
call('/api/v1/me',token=app,method='GET');assert call('/api/v1/catalog?limit=50',token=app,method='GET')['items']
# Cancel before approval; browser credentials cannot resurrect a cancelled grant.
f=start();t=login(f)['token'];assert poll(f,'?cancel=true')['state']=='cancelled'
call(f['base']+'/approve',{'browser_key':f['key']},t,status=409)
call('/api/v1/auth/logout',{},t,status=204)
# Cancel after approval never mints an app token.
f=start();t=login(f)['token'];approve(f,t);assert poll(f,'?cancel=true')['state']=='cancelled';assert poll(f)['state']=='cancelled'
# Contact verification is tied to the app's account; other accounts cannot finish it.
f=start('email',app);wrong=login(f,'linzhou@liyu.test')['token']
call(f['base']+'/approve',{'browser_key':f['key']},wrong,status=409);call('/api/v1/auth/logout',{},wrong,status=204)
t=login(f)['token'];value='auth-'+uuid.uuid4().hex+'@liyu.test'
c=call('/api/v1/auth/challenges',{'kind':'email','value':value,'purpose':'bind'},t)
call('/api/v1/me/email',{'value':value,'code':c['test_code'],'challenge_id':c['challenge_id']},t,method='PUT')
approve(f,t);assert poll(f)=={'state':'complete'}
assert call('/api/v1/me/profile',token=app,method='GET')['email']==value
# Register from the browser, then issue a separate app session.
f=start();identity='new-'+uuid.uuid4().hex+'@liyu.test'
c=call('/api/v1/auth/challenges',{'kind':'email','value':identity,'purpose':'register'})
t=call(f['base']+'/login?mode=register',{'browser_key':f['key'],'identifier':identity,'password':'test-password-123','display_name':'授权测试','code':c['test_code'],'challenge_id':c['challenge_id']})['token']
approve(f,t);new=poll(f)['token'];call('/api/v1/me',token=new,method='GET');call('/api/v1/auth/logout',{},new,status=204)
# Expired flow cannot be used with either nonce.
if os.getenv('DATABASE_URL'):
 f=start(); subprocess.run(['psql',os.environ['DATABASE_URL'],'-c',"UPDATE browser_authorizations SET expires_at=now()-interval '1 second' WHERE id='"+f['id']+"'"],check=True,capture_output=True)
 assert poll(f)['state']=='expired';call(f['base']+'/info',{'browser_key':f['key']},status=410)
# Bound browser password attempts stop after ten failures.
f=start()
for _ in range(10): call(f['base']+'/login',{'browser_key':f['key'],'identifier':'demo@liyu.test','password':'wrong'},status=401)
call(f['base']+'/login',{'browser_key':f['key'],'identifier':'demo@liyu.test','password':'123456'},status=429)
call('/api/v1/auth/logout',{},app,status=204);call('/api/v1/me',token=app,method='GET',status=401)
print(f'Browser authorization: {checks} HTTP checks passed')
