"""Draft cart, publication boundary, and claims; disposable database only."""
import datetime as dt
import json,os,subprocess,sys,urllib.request,urllib.error,urllib.parse,uuid
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
base=sys.argv[1] if len(sys.argv)>1 else 'http://127.0.0.1:18846'
dsn=os.environ['DATABASE_URL']
assert urllib.parse.urlsplit(base).hostname=='127.0.0.1'
assert 'liyu_wishlist_test_' in urllib.parse.urlsplit(dsn).path
checks=0

def call(path,method='GET',data=None,token=None,status=200):
 global checks
 h={'Content-Type':'application/json'}
 if token:h['Authorization']='Bearer '+token
 q=urllib.request.Request(base+'/api/v1'+path,method=method,headers=h,data=json.dumps(data).encode() if data is not None else None)
 try:r=urllib.request.urlopen(q,timeout=10)
 except urllib.error.HTTPError as error:r=error
 body=r.read();assert r.status==status,(path,r.status,status,body[:300]);checks+=1
 return json.loads(body) if body else None

def sql(query):
 r=subprocess.run(['psql','-d',dsn,'-XAt','-v','ON_ERROR_STOP=1','-c',query],capture_output=True,text=True)
 assert r.returncode==0,'scratch SQL failed'
 return r.stdout.strip()

auth=[call('/auth/login','POST',{'identifier':i,'password':'123456'}) for i in ['demo@liyu.test','linzhou@liyu.test','chenxiao@liyu.test']]
a,b,c=[v['token'] for v in auth];uid=call('/me',token=a)['id'];friend=call('/me',token=b)['id']
date=dt.datetime.now(dt.timezone.utc).strftime('%Y-%m-%d')
header={'title':'多件生日心愿','note':'整理后再发布','occasion':'birthday','event_on':date,'expires_hours':24,'audience_user_ids':[friend]}
call('/wishlists/drafts','POST',{'title':'未登录'},status=401)
wid=call('/wishlists/drafts','POST',{'title':'草稿验证'},a)['id'];path=f'/wishlists/{wid}'
assert call(path,token=a)['status']=='draft' and call(path,token=a)['items']==[]
call(path+'/publish','POST',header,a,409)
item=call(path+'/items','POST',{'product_id':0},a)
assert not item['already_present']
assert call(path+'/items','POST',{'product_id':0},a)['already_present']
assert call(path,token=a)['item_count']==1
for other in (b,c):
 call(path,token=other,status=404)
 assert not any(w['id']==wid for w in call('/wishlists/friends',token=other))
 call(path+'/items','POST',{'product_id':1},other,409)
 call(path+'/publish','POST',header,other,404)
call('/orders/quote','POST',{'product_id':0,'recipient_id':uid,'wish_item_id':item['id']},b,409)
# Empty drafts are useful carts; the last item can be removed.
call(path+f"/items/{item['id']}",'DELETE',token=a,status=204)
assert call(path,token=a)['items']==[]
# Old metadata does not age or expire a private draft.
sql(f"UPDATE wishlists SET event_on=CURRENT_DATE-20,expires_at=now()-interval '10 days' WHERE id={wid}")
call(path+'/items','POST',{'product_id':0},a)
call(path,'PUT',header,a)
with ThreadPoolExecutor(max_workers=2) as pool:
 list(pool.map(lambda pid:call(path+'/items','POST',{'product_id':pid},a),[1,2]))
assert len(call(path,token=a)['items'])==3
# Concurrent duplicate additions collapse to a single cart item.
with ThreadPoolExecutor(max_workers=2) as pool:
 results=list(pool.map(lambda _:call(path+'/items','POST',{'product_id':3},a),range(2)))
assert results[0]['id']==results[1]['id'] and sum(x['already_present'] for x in results)==1
for pid in range(4,8):call(path+'/items','POST',{'product_id':pid},a)
call(path+'/items','POST',{'product_id':8},a,409)
assert len(call(path,token=a)['items'])==8
before=int(sql(f'SELECT extract(epoch FROM expires_at)::bigint FROM wishlists WHERE id={wid}'))
call(path+'/publish','POST',dict(header,audience_user_ids=[uid]),a,409)
assert call(path,token=a)['status']=='draft'
call(path+'/publish','POST',header,a)
published=call(path,token=a)
assert published['status']=='published' and published['is_open'] and len(published['items'])==8
assert published['expires_at']>=before
assert call(path,token=b)['status']=='published'
call(path,token=c,status=404)
assert any(w['id']==wid for w in call('/wishlists/friends',token=b))
assert not any(w['id']==wid for w in call('/wishlists/friends',token=c))
call(path+'/items','POST',{'product_id':8},a,409)
call(path+f"/items/{published['items'][0]['id']}",'DELETE',token=a,status=409)
call('/orders/quote','POST',{'product_id':0,'recipient_id':uid,'wish_item_id':published['items'][0]['id']},b)
expiry=published['expires_at'];call(path+'/publish','POST',dict(header,expires_hours=72),a)
assert call(path,token=a)['expires_at']==expiry # idempotent publication, no extension
call(path+'/close','POST',{},a)
call(path+'/publish','POST',header,a,409)
# Saving a selected product with a new list is atomic; legacy lists stay published.
new=call('/wishlists/drafts','POST',{'title':'选中商品新建','items':[{'product_id':9}]},a)['id']
assert call(f'/wishlists/{new}',token=a)['items'][0]['product_id']==9
call('/wishlists/drafts','POST',{'title':'不存在的商品','items':[{'product_id':999999}]},a,400)
legacy=call('/wishlists','POST',dict(header,items=[{'product_id':10}]),a)['id']
assert call(f'/wishlists/{legacy}',token=a)['status']=='published'
# Omitting an audience on publish preserves an already saved private selection.
private=call('/wishlists/drafts','POST',{'title':'预设可见范围','items':[{'product_id':12}]},a)['id']
call(f'/wishlists/{private}','PUT',header,a)
no_audience={k:v for k,v in header.items() if k!='audience_user_ids'}
call(f'/wishlists/{private}/publish','POST',no_audience,a)
assert call(f'/wishlists/{private}',token=a)['audience_user_ids']==[friend]
call(f'/wishlists/{private}',token=c,status=404)
# Last removal and publication are serialized on the parent list.
race=call('/wishlists/drafts','POST',{'title':'并发发布','items':[{'product_id':11}]},a)['id']
ri=call(f'/wishlists/{race}',token=a)['items'][0]['id']
def attempt(path,method,data=None):
 h={'Authorization':'Bearer '+a,'Content-Type':'application/json'}
 q=urllib.request.Request(base+'/api/v1'+path,method=method,headers=h,data=json.dumps(data).encode() if data else None)
 try:r=urllib.request.urlopen(q)
 except urllib.error.HTTPError as error:r=error
 r.read();return r.status
with ThreadPoolExecutor(max_workers=2) as pool:
 futures=[pool.submit(attempt,f'/wishlists/{race}/publish','POST',header),pool.submit(attempt,f'/wishlists/{race}/items/{ri}','DELETE')]
 outcomes=[f.result() for f in futures]
assert outcomes in ([200,409],[409,204]),outcomes
end=call(f'/wishlists/{race}',token=a)
assert end['status']=='draft' or len(end['items'])>=1
root=os.environ.get('LIYU_WISHLIST_UI_DIR')
if root:
 dest=Path(root)/'app/.host/liyu/liyu-mini/session.json';dest.parent.mkdir(parents=True,exist_ok=True)
 # A separate owner starts with no drafts, for the empty-state and create flow.
 session=auth[1];dest.write_text(json.dumps({'token':session['token'],'identifier':session['user']['identifier']}));dest.chmod(0o600)
print(f'PASS: {checks} HTTP checks; multi-item carts, privacy, duplicate/concurrent additions, expiry, atomic/idempotent publication, published claims, legacy compatibility')
