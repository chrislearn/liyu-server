"""Real HTTP/DB operations against a disposable server. No production credentials."""
import datetime as dt
import http.cookiejar
import json
import os
import sys
import urllib.error
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
base=sys.argv[1] if len(sys.argv)>1 else 'http://127.0.0.1:18801'
admin=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
plain=urllib.request.build_opener()
csrf=''
checks=0

def call(path,method='GET',data=None,token=None,status=200,protect=True):
    global checks
    headers={'Content-Type':'application/json'}
    if protect:
        headers['X-Admin-Request']='1'
        if csrf: headers['X-CSRF-Token']=csrf
    if token: headers['Authorization']='Bearer '+token
    req=urllib.request.Request(base+path,data=json.dumps(data).encode() if data is not None else None,headers=headers,method=method)
    try: r=(plain if token else admin).open(req)
    except urllib.error.HTTPError as e: r=e
    body=r.read()
    assert r.status==status,(path,r.status,status,body[:500])
    checks+=1
    return json.loads(body) if 'application/json' in r.headers.get('Content-Type','') else body

def manage(action,id,**fields):
    return call(f'/admin/api/manage/{action}/{id}','POST',dict(reason='management E2E',**fields))

def report(module,q=''):
    return call('/admin/api/reports/'+module+'?q='+urllib.parse.quote(q))['items']

call('/admin/api/overview',status=401)
csrf=call('/admin/api/login','POST',{'username':os.environ['LIYU_TEST_ADMIN_USERNAME'],'password':os.environ['LIYU_TEST_ADMIN_PASSWORD']})['csrf_token']
for module in ['users','inventory','low-stock','prices','history','coupons','issued-coupons','recycling','wallet','orders','gifts','shipments','friends','wishlists','contracts','notifications','audit']:
    report(module)
call('/admin/api/manage/stock/0','POST',{'delta':1},status=403,protect=False)
call('/admin/api/manage/stock/0','POST',{'delta':1},status=400)
call('/admin/api/reports/not-a-report',status=404)
assert call('/admin/api/overview')['users']>=3
users=report('users')
a=next(u for u in users if u['identifier']=='demo@liyu.test')['id']
b=next(u for u in users if u['identifier']=='linzhou@liyu.test')['id']
c=next(u for u in users if u['identifier']=='chenxiao@liyu.test')['id']
assert all('password_hash' not in u and 'state' not in u for u in users)
token=call('/api/v1/auth/login','POST',{'identifier':'demo@liyu.test','password':'123456'})['token']
recipient=call('/api/v1/auth/login','POST',{'identifier':'linzhou@liyu.test','password':'123456'})['token']
manage('user-status',c,is_active=False)
call('/api/v1/auth/login','POST',{'identifier':'chenxiao@liyu.test','password':'123456'},status=401)
manage('user-status',c,is_active=True)
other=call('/api/v1/auth/login','POST',{'identifier':'chenxiao@liyu.test','password':'123456'})['token']
manage('user-status',c,is_active=False)
call('/api/v1/me',token=other,status=401)
manage('user-status',c,is_active=True)
p={'name':'Operations '+str(uuid.uuid4()),'category':'home','price_cents':10000,'physical':True,'brand':'测试','kind':'礼品','spec':'一件','description':'管理测试','tags':['test'],'stock':10,'is_active':True}
pid=call('/admin/api/products','POST',p)['id']
manage('stock',pid,delta=3)
assert next(r for r in report('inventory') if r['id']==pid)['stock']==13
call(f'/admin/api/manage/stock/{pid}','POST',{'delta':-100,'reason':'underflow'},status=409)
manage('price',pid,price_cents=12000)
manage('listing',pid,is_active=False)
call(f'/api/v1/catalog/{pid}',status=404)
manage('listing',pid,is_active=True)
assert len([r for r in report('history') if r['product_id']==pid])==4
now=dt.datetime.now(dt.timezone.utc)
def date(days): return (now+dt.timedelta(days=days)).isoformat()
template={'name':'满减测试 '+str(uuid.uuid4()),'kind':'promotion','discount_kind':'fixed','value':2000,'min_spend_cents':5000,'max_discount_cents':0,'product_id':pid,'category':'home','starts_at':date(-1),'expires_at':date(1),'issue_limit':2,'per_user_limit':1,'is_active':True}
tid=manage('coupon',0,**template)['id']
issued=manage('coupon-issue',tid,user_id=a)['id']
call(f'/admin/api/manage/coupon-issue/{tid}','POST',{'user_id':a,'reason':'quota'},status=409)
call('/api/v1/me/coupons',token=token)
# An API helper with explicit idempotency and coupon headers.
def user_order(path,method='POST',data=None,key=None,coupon=None,status=200):
    global checks
    if data is None and path in ('/api/v1/orders','/api/v1/orders/quote'):data={'product_id':pid,'recipient_id':b}
    h={'Authorization':'Bearer '+token,'Content-Type':'application/json'}
    if key:h['Idempotency-Key']=key
    if coupon:h['X-Coupon-Id']=str(coupon)
    req=urllib.request.Request(base+path,method=method,headers=h,data=json.dumps(data).encode() if data is not None else None)
    try:r=plain.open(req)
    except urllib.error.HTTPError as e:r=e
    body=r.read();assert r.status==status,(path,r.status,status,body[:500]);checks+=1
    return json.loads(body)
quote=user_order('/api/v1/orders/quote',coupon=issued)
assert quote['total_cents']==10000 and quote['discount_cents']==2000
key=str(uuid.uuid4())
order=user_order('/api/v1/orders',key=key,coupon=issued)
assert order['total_cents']==10000
assert user_order('/api/v1/orders',key=key,coupon=issued)['id']==order['id']
manage('price',pid,price_cents=15000)
paid=user_order(f"/api/v1/orders/{order['id']}/pay-test")
assert paid['total_cents']==10000
user_order(f"/api/v1/orders/{order['id']}/pay-test")
assert next(r for r in report('issued-coupons') if r['id']==issued)['status']=='used'
call(f'/admin/api/manage/cancel-order/{order["id"]}','POST',{'reason':'paid cancel'},status=409)
gift=call(f'/api/v1/orders/{order["id"]}',token=token)['items'][0]['gift_id']
assert next(r for r in report('gifts') if r['id']==gift)['price_cents']==10000
manage('recycle',pid,is_active=True,mode='percentage',value=9000,expires_at=None)
call(f'/api/v1/gifts/{gift}/puzzle','PUT',{'unlock_kind':'free'},token=token)
call(f'/api/v1/gifts/{gift}/open','POST',token=recipient)
assert call(f'/api/v1/gifts/{gift}/recovery-quote',token=recipient)['recovery_cents']==9000
call(f'/api/v1/gifts/{gift}/cash-out','POST',token=other,status=401) # disabled token
before=call('/api/v1/wallet',token=recipient)['balance_cents']
settled=call(f'/api/v1/gifts/{gift}/cash-out','POST',token=recipient)
assert call(f'/api/v1/gifts/{gift}/cash-out','POST',token=recipient)==settled
assert call('/api/v1/wallet',token=recipient)['balance_cents']==before+9000
assert sum(r['gift_id']==gift for r in report('wallet'))==1
# Cancellation releases a reserved coupon; revocation is allowed only when available.
issued_b=manage('coupon-issue',tid,user_id=b)['id']
manage('coupon-revoke',issued_b)
manage('coupon-status',tid,is_active=False)
call(f'/admin/api/manage/coupon-issue/{tid}','POST',{'user_id':c,'reason':'disabled'},status=409)
# Refunds are service owned and stock is restored once.
order2=user_order('/api/v1/orders',key=str(uuid.uuid4()))
user_order(f"/api/v1/orders/{order2['id']}/pay-test")
gift2=call(f'/api/v1/orders/{order2["id"]}',token=token)['items'][0]['gift_id']
balance=call('/api/v1/wallet',token=token)['balance_cents']
call(f'/api/v1/gifts/{gift2}/withdraw','POST',token=token)
assert call('/api/v1/wallet',token=token)['balance_cents']==balance+15000
call(f'/api/v1/gifts/{gift2}/withdraw','POST',token=token,status=409)
# Shipping cannot start for a sealed gift, and never writes recipient confirmation.
ship={'carrier':'测试快递','tracking_number':str(uuid.uuid4()),'recipient_name':'测试','recipient_phone':'000','recipient_address':'测试路','description':'已揽收','delivered':False}
call(f'/admin/api/manage/shipment/{gift2}','POST',dict(ship,reason='invalid shipment'),status=409)
manage('shipment',900003,**ship)
manage('shipment',900003,**dict(ship,description='已签收',delivered=True))
assert next(r for r in report('shipments') if r['id']==900003)['recipient_confirmed_at'] is None
manage('close-wishlist',900103)
notif=manage('notify',0,user_id=a,title='管理测试',body='仅自己可见',expires_at=date(1))['id']
assert any(r['id']==notif for r in call('/api/v1/notifications',token=token))
assert not any(r['id']==notif for r in call('/api/v1/notifications',token=recipient))
call(f'/api/v1/notifications/{notif}/read','POST',token=recipient,status=404)
call(f'/api/v1/notifications/{notif}/read','POST',token=token)
manage('revoke-notification',notif)
assert not any(r['id']==notif for r in call('/api/v1/notifications',token=token))
assert any(r['action']=='coupon' for r in report('audit'))
# Rule validation, expired/future windows, first-order eligibility and percentage caps.
call('/admin/api/manage/coupon/0','POST',dict(template,reason='invalid dates',expires_at=date(-2)),status=400)
call('/admin/api/manage/coupon/0','POST',dict(template,reason='invalid percentage',discount_kind='percentage',value=10001),status=400)
expired=manage('coupon',0,**dict(template,name='Expired',starts_at=date(-3),expires_at=date(-1)))['id']
call(f'/admin/api/manage/coupon-issue/{expired}','POST',{'user_id':a,'reason':'expired'},status=409)
new_user=manage('coupon',0,**dict(template,name='New user',kind='new_user'))['id']
call(f'/admin/api/manage/coupon-issue/{new_user}','POST',{'user_id':a,'reason':'not new'},status=409)
future=manage('coupon',0,**dict(template,name='Future',starts_at=date(10),expires_at=date(20)))['id']
future_coupon=manage('coupon-issue',future,user_id=a)['id']
user_order('/api/v1/orders/quote',coupon=future_coupon,status=409)
percentage=manage('coupon',0,**dict(template,name='Percentage capped',discount_kind='percentage',value=1000,max_discount_cents=500))['id']
percentage_coupon=manage('coupon-issue',percentage,user_id=a)['id']
assert user_order('/api/v1/orders/quote',coupon=percentage_coupon)['discount_cents']==500
# Concurrent stock adjustments cannot oversell.
stock=next(r for r in report('inventory') if r['id']==pid)['stock']
manage('stock',pid,delta=1-stock)
def raw_manage(action,ident,fields):
    req=urllib.request.Request(base+f'/admin/api/manage/{action}/{ident}',data=json.dumps(dict(reason='race',**fields)).encode(),method='POST',headers={'Content-Type':'application/json','X-Admin-Request':'1','X-CSRF-Token':csrf})
    try:r=admin.open(req)
    except urllib.error.HTTPError as e:r=e
    return r.status,json.loads(r.read())
with ThreadPoolExecutor(max_workers=2) as executor:
    raced=list(executor.map(lambda _:raw_manage('stock',pid,{'delta':-1}),range(2)))
assert sorted(code for code,_ in raced)==[200,409]
manage('stock',pid,delta=10)
tid2=manage('coupon',0,**dict(template,name='Race coupon',issue_limit=1))['id']
with ThreadPoolExecutor(max_workers=2) as executor:
    raced=list(executor.map(lambda _:raw_manage('coupon-issue',tid2,{'user_id':a}),range(2)))
assert sorted(code for code,_ in raced)==[200,409]
cancel_coupon=next(data['id'] for code,data in raced if code==200)
cancel_order=user_order('/api/v1/orders',key=str(uuid.uuid4()),coupon=cancel_coupon)
manage('cancel-order',cancel_order['id'])
assert next(r for r in report('issued-coupons') if r['id']==cancel_coupon)['status']=='available'
exchange_order=user_order('/api/v1/orders',key=str(uuid.uuid4()))
user_order(f"/api/v1/orders/{exchange_order['id']}/pay-test")
exchange_gift=call(f"/api/v1/orders/{exchange_order['id']}",token=token)['items'][0]['gift_id']
call(f'/api/v1/gifts/{exchange_gift}/puzzle','PUT',{'unlock_kind':'free'},token=token)
call(f'/api/v1/gifts/{exchange_gift}/open','POST',token=recipient)
manage('recycle',pid,is_active=False,mode='fixed',value=999999,expires_at=None)
call(f'/api/v1/gifts/{exchange_gift}/recovery-quote',token=recipient,status=409)
manage('recycle',pid,is_active=True,mode='fixed',value=999999,expires_at=None)
assert call(f'/api/v1/gifts/{exchange_gift}/recovery-quote',token=recipient)['recovery_cents']==15000
manage('recycle',pid,is_active=True,mode='percentage',value=9000,expires_at=None)
balance=call('/api/v1/wallet',token=recipient)['balance_cents']
with ThreadPoolExecutor(max_workers=2) as executor:
    list(executor.map(lambda _:call(f'/api/v1/gifts/{exchange_gift}/exchange','POST',{'product_id':pid},token=recipient),range(2)))
assert call('/api/v1/wallet',token=recipient)['balance_cents']==balance-1500
assert sum(r['gift_id']==exchange_gift for r in report('wallet'))==1
contract_order=user_order('/api/v1/orders',key=str(uuid.uuid4()))
user_order(f"/api/v1/orders/{contract_order['id']}/pay-test")
contract_gift=call(f"/api/v1/orders/{contract_order['id']}",token=token)['items'][0]['gift_id']
call(f'/api/v1/gifts/{contract_gift}/puzzle','PUT',{'unlock_kind':'free','contract_text':'一起喝杯咖啡'},token=token)
call(f'/admin/api/manage/contract/{contract_gift}','POST',{'reason':'personal status only','status':'fulfilled'},status=400)
call(f'/api/v1/gifts/{contract_gift}/open','POST',token=recipient)
call(f'/api/v1/gifts/{contract_gift}/accept','POST',{'agree':True,'recipient_name':'测试','recipient_phone':'13800138000','recipient_address':'测试地址'},token=recipient)
call(f'/api/v1/contracts/{contract_gift}/status','PUT',{'status':'fulfilled'},token=token)
contract_report=next(r for r in report('contracts') if r['id']==contract_gift)
assert contract_report['sender_status']=='fulfilled' and contract_report['recipient_status']=='pending'
call(f'/admin/api/manage/contract/{contract_gift}','POST',{'reason':'personal status only','status':'fulfilled'},status=400)
other=call('/api/v1/auth/login','POST',{'identifier':'chenxiao@liyu.test','password':'123456'})['token']
manage('user-sessions',c)
call('/api/v1/me',token=other,status=401)
relation=next(r for r in report('friends') if {r['user_low_id'],r['user_high_id']}=={a,c})['id']
manage('remove-friend',relation)
assert not any(r['id']==relation for r in report('friends'))
call('/admin/api/logout','POST',status=204)
print(f'Management integration: {checks} HTTP checks passed.')
