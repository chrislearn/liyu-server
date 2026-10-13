"""Large native-host pages and Shanghai business dates on a disposable database."""
import json, os, subprocess, sys, urllib.error, urllib.parse, urllib.request
local_http=urllib.request.build_opener(urllib.request.ProxyHandler({}))
base=sys.argv[1]; dsn=os.environ['DATABASE_URL']; checks=0
assert urllib.parse.urlsplit(base).hostname=='127.0.0.1'
assert 'liyu_resilience_test_' in urllib.parse.urlsplit(dsn).path

def sql(query):
    return subprocess.check_output(['psql',dsn,'-XAt','-v','ON_ERROR_STOP=1','-c',query],text=True).strip()
def call(path, token='', data=None, status=200):
    global checks
    req=urllib.request.Request(base+'/api/v1'+path,headers={'Content-Type':'application/json','Authorization':'Bearer '+token},data=None if data is None else json.dumps(data).encode())
    try:r=local_http.open(req,timeout=20)
    except urllib.error.HTTPError as e:r=e
    raw=r.read();assert r.status==status,(path,r.status,raw[:200]);checks+=1
    return json.loads(raw)
token=call('/auth/login',data={'identifier':'demo@liyu.test','password':'123456'})['token']
uid=call('/me',token)['id']; other=sql("SELECT id FROM users WHERE identifier='linzhou@liyu.test'")
sql(f"INSERT INTO users(id,identifier,display_name,password_hash) SELECT 1100000+i,'page-'||i||'@test.invalid','分页熟人'||i,'not-a-password' FROM generate_series(1,61) i; INSERT INTO friendships(user_low_id,user_high_id,requested_by_user_id,status,accepted_at) SELECT LEAST({uid},id),GREATEST({uid},id),{uid},'accepted',now() FROM users WHERE id BETWEEN 1100001 AND 1100061")
sql(f"INSERT INTO gifts(sender_id,recipient_id,product_id,price_cents,state,puzzle_configured) SELECT {other},{uid},0,3500,'sealed',true FROM generate_series(1,125)")
sql("UPDATE catalog SET description=repeat('真实的长商品说明。',200) WHERE id=0")
sql("INSERT INTO catalog(id,name,category,price_cents,physical,brand,kind,spec,description,tags,stock,is_active) SELECT 1100000+i,'分页商品'||i,'coffee',3500,false,'测试','咖啡','测试规格','测试说明',ARRAY[]::text[],99,true FROM generate_series(1,61) i")

def pages(path):
    cursor='0'; ids=[]
    for _ in range(300):
        target=path+'?limit=20&before='+cursor
        result=call('/host/read?'+urllib.parse.urlencode({'path':target}),token)
        assert result['status']==200,result
        page=result['body']; assert len(json.dumps(page,ensure_ascii=False).encode())<60000
        batch=[row.get('id',row.get('gift_id')) for row in page['items']]
        assert len(batch)<=20 and len(set(batch))==len(batch)
        assert all(a>b for a,b in zip(batch,batch[1:]))
        if ids and batch:assert ids[-1]>batch[0]
        ids+=batch
        if page['next_cursor'] is None:return ids
        assert batch and page['next_cursor']==str(batch[-1]);cursor=page['next_cursor']
    raise AssertionError('cursor did not finish')
friend_ids=pages('/friends');assert len(friend_ids)>=61 and len(set(friend_ids))==len(friend_ids)
inbox_ids=pages('/gifts/inbox');assert len(inbox_ids)>=125 and len(set(inbox_ids))==len(inbox_ids)
sql(f"INSERT INTO gifts(sender_id,recipient_id,product_id,price_cents,state,contract_text,created_at) VALUES({other},{uid},0,3500,'accepted','跨午夜约定','2026-10-12 16:30:00+00')")
contracts=call('/contracts?limit=20',token)['items']
assert any(c['contract_text']=='跨午夜约定' and c['created_on']=='2026-10-13' for c in contracts)
for path in ['/gifts/outbox' ,'/contracts','/wishlists/mine','/wishlists/friends','/notifications']:pages(path)
call('/friends?limit=0',token,status=400);call('/friends?limit=21',token,status=400);call('/friends?limit=10&before=-1',token,status=400)
assert isinstance(call('/friends',token),list),'legacy array contract preserved'
cursor='-1';products=[]
while True:
    result=call('/host/read?'+urllib.parse.urlencode({'path':'/catalog?limit=20&cursor='+cursor}),token)
    assert result['status']==200
    products+=result['body']['items'];cursor=result['body']['next_cursor']
    if cursor is None:break
assert len(products)>50 and any(p['id']==1100061 for p in products)
# Current calendar-day bounds are explicitly Shanghai, even when the DB session uses another zone.
last_day=sql("SELECT ((now() AT TIME ZONE 'Asia/Shanghai')::date+180)::text")
call('/wishlists',token,data={'title':'上海日期边界','event_on':last_day,'items':[{'product_id':0}]})
expired=sql("SELECT ((now() AT TIME ZONE 'Asia/Shanghai')::date-1)::text")
call('/wishlists',token,data={'title':'昨天不发布','event_on':expired,'items':[{'product_id':0}]},status=400)
print(f'RESILIENCE_HTTP_CHECKS={checks}; friends={len(friend_ids)}; inbox={len(inbox_ids)}; products={len(products)}')
