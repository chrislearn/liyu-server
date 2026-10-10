#!/usr/bin/env python3
"""Native PKCE contract on a disposable database; no live host approval claim."""
import base64, hashlib, json, os, re, subprocess, sys, urllib.request, urllib.error, urllib.parse
from concurrent.futures import ThreadPoolExecutor
base=sys.argv[1]; checks=0

def call(path,body=None,token='',status=200,method='POST',form=False):
 global checks
 headers={'Content-Type':'application/x-www-form-urlencoded' if form else 'application/json'}
 if token: headers['Authorization']='Bearer '+token
 data=None if body is None else (urllib.parse.urlencode(body).encode() if form else json.dumps(body).encode())
 try:r=urllib.request.urlopen(urllib.request.Request(base+path,data=data,headers=headers,method=method),timeout=30)
 except urllib.error.HTTPError as e:r=e
 raw=r.read(); assert r.status==status,(path,r.status,status,raw[:200]); checks+=1
 return json.loads(raw) if raw else {}
verifier='a'*64
challenge=base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
params={'client_id':'liyu-mini-host-v1','response_type':'code','scope':'app.session','code_challenge_method':'S256','code_challenge':challenge,'state':'liyu-state-test-0123456789','redirect_uri':'https://octosense.invalid/auth/callback'}

def authorize(redirect=None):
 query=dict(params)
 if redirect:query['redirect_uri']=redirect
 r=urllib.request.urlopen(base+'/oauth/authorize?'+urllib.parse.urlencode(query))
 html=r.read().decode();assert r.headers['Cache-Control']=='no-store'
 match=re.search(r'const \$=id=>document.getElementById\(id\), id=("[^"\n]+"), key=("[^"\n]+")',html);assert match
 id,key=map(json.loads,match.groups());path='/api/v1/browser-authorizations/'+id
 temp=call(path+'/login',{'browser_key':key,'identifier':'demo@liyu.test','password':'123456'})['token']
 result=call(path+'/approve',{'browser_key':key},temp)
 call('/oauth/me',token=temp,method='GET',status=401)
 returned=urllib.parse.parse_qs(urllib.parse.urlsplit(result['redirect_uri']).query);assert returned['state']==[params['state']]
 return {'grant_type':'authorization_code','client_id':params['client_id'],'redirect_uri':query['redirect_uri'],'code':returned['code'][0],'code_verifier':verifier},path
for changed in [{'redirect_uri':'https://evil.test/callback'},{'scope':'all'},{'code_challenge_method':'plain'},{'state':'x'}]:
 call('/oauth/authorize?'+urllib.parse.urlencode(dict(params,**changed)),method='GET',status=400)
grant,path=authorize();call('/oauth/token',dict(grant,code_verifier='b'*64),form=True,status=400)
call('/oauth/token',dict(grant,redirect_uri='http://127.0.0.1:4444/oauth/callback'),form=True,status=400)
first=call('/oauth/token',grant,form=True);assert first['token_type']=='Bearer' and first['expires_in']==3600
call('/oauth/token',grant,form=True,status=400)
identity=call('/oauth/me',token=first['access_token'],method='GET');assert identity['sub'] and identity['label']
r=call('/api/v1/host/read?'+urllib.parse.urlencode({'path':'/catalog?limit=50'}),token=first['access_token'],method='GET');assert r['status']==200 and r['body']['items']
# Exercise the exact encoded path used by the mini-app's host-managed read transport.
options=call('/api/v1/host/read?'+urllib.parse.urlencode({'path':'/catalog/search-options'}),token=first['access_token'],method='GET')
assert options['status']==200 and options['body']['tool']['name']=='catalog_search'
filter={'max_price_cents':3000,'allowed_kinds':['咖啡'],'excluded_kinds':['奶茶'],'delivery':'any'}
path='/catalog/search?'+urllib.parse.urlencode({'filter':json.dumps(filter,ensure_ascii=False),'q':'','cursor':-1,'limit':20})
result=call('/api/v1/host/read?'+urllib.parse.urlencode({'path':path}),token=first['access_token'],method='GET')
assert result['status']==200 and result['body']['eligible_total']==0 and result['body']['applied']==filter
for path in ['/oauth/token','/me/../auth/login','//evil.test','/me/%2e%2e/auth']:
 call('/api/v1/host/read?'+urllib.parse.urlencode({'path':path}),token=first['access_token'],method='GET',status=400)
call('/api/v1/host/post',{'path':'/browser-authorizations','body':{'purpose':'login'}},first['access_token'],status=400)
flow=call('/api/v1/host/post',{'path':'/browser-authorizations','body':{'purpose':'email'}},first['access_token'])['body']
status=call('/api/v1/host/contact-status?'+urllib.parse.urlencode({'path':'/api'.replace('/api','')+'/browser-authorizations/'+flow['id']+'/poll','poll_key':flow['poll_key']}),token=first['access_token'],method='GET');assert status['body']['state']=='pending'
call('/api/v1/host/contact-status?'+urllib.parse.urlencode({'path':'/browser-authorizations/not-yours/poll','poll_key':'wrong'}),token=first['access_token'],method='GET',status=403)
second=call('/oauth/token',{'grant_type':'refresh_token','client_id':params['client_id'],'refresh_token':first['refresh_token']},form=True)
call('/oauth/me',token=first['access_token'],method='GET',status=401)
call('/oauth/me',token=second['access_token'],method='GET')
call('/oauth/token',{'grant_type':'refresh_token','client_id':params['client_id'],'refresh_token':first['refresh_token']},form=True,status=400)
call('/oauth/me',token=second['access_token'],method='GET',status=401)
call('/oauth/token',{'grant_type':'refresh_token','client_id':params['client_id'],'refresh_token':second['refresh_token']},form=True,status=400)
grant,path=authorize('http://127.0.0.1:44001/oauth/callback')
with ThreadPoolExecutor(max_workers=2) as executor:
 def exchange(_):
  try:return urllib.request.urlopen(urllib.request.Request(base+'/oauth/token',data=urllib.parse.urlencode(grant).encode(),headers={'Content-Type':'application/x-www-form-urlencoded'})).status
  except urllib.error.HTTPError as e:return e.code
 results=list(executor.map(exchange,range(2)))
assert sorted(results)==[200,400]
grant,path=authorize();race=call('/oauth/token',grant,form=True)
refresh={'grant_type':'refresh_token','client_id':params['client_id'],'refresh_token':race['refresh_token']}
def rotate(_):
 try:r=urllib.request.urlopen(urllib.request.Request(base+'/oauth/token',data=urllib.parse.urlencode(refresh).encode(),headers={'Content-Type':'application/x-www-form-urlencoded'}));return r.status,json.load(r)
 except urllib.error.HTTPError as e:return e.code,{}
with ThreadPoolExecutor(max_workers=2) as executor:rotations=list(executor.map(rotate,range(2)))
assert sorted(code for code,_ in rotations)==[200,400]
winner=next(body for code,body in rotations if code==200)
call('/oauth/me',token=winner['access_token'],method='GET',status=401)
grant,path=authorize();third=call('/oauth/token',grant,form=True)
call('/oauth/logout',{},third['access_token']);call('/oauth/me',token=third['access_token'],method='GET',status=401)
call('/oauth/token',{'grant_type':'refresh_token','client_id':params['client_id'],'refresh_token':third['refresh_token']},form=True,status=400)
grant,path=authorize();subprocess.run(['psql',os.environ['DATABASE_URL'],'-c',"UPDATE host_oauth_requests SET expires_at=now()-interval '1 second' WHERE auth_id='"+path.rsplit('/',1)[1]+"'"],check=True,capture_output=True)
call('/oauth/token',grant,form=True,status=400)
print(f'Host PKCE: {checks} HTTP checks and concurrent single-use exchange passed')
