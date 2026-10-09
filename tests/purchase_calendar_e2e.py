"""Independent promise marks through real APIs, on a disposable database only."""
import json
import os
from pathlib import Path
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor

base = sys.argv[1] if len(sys.argv) > 1 else 'http://127.0.0.1:18845'
dsn = os.environ['DATABASE_URL']
assert urllib.parse.urlsplit(base).hostname == '127.0.0.1'
assert 'liyu_purchase_test_' in urllib.parse.urlsplit(dsn).path
checks = 0

def call(path, method='GET', data=None, token=None, status=200, key=None, expected=None):
    global checks
    headers = {'Content-Type': 'application/json'}
    if token: headers['Authorization'] = 'Bearer ' + token
    if key: headers['Idempotency-Key'] = key
    if expected is not None: headers['X-Expected-Total-Cents'] = str(expected)
    req = urllib.request.Request(base + '/api/v1' + path, method=method, headers=headers,
        data=json.dumps(data).encode() if data is not None else None)
    try: response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error: response = error
    body = response.read()
    assert response.status == status, (path, response.status, status, body[:300])
    checks += 1
    return json.loads(body) if body else None

def sql(query):
    r = subprocess.run(['psql', '-d', dsn, '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', query],
        capture_output=True, text=True)
    assert r.returncode == 0, 'scratch SQL failed'
    return r.stdout.strip()

sender = call('/auth/login', 'POST', {'identifier': 'demo@liyu.test', 'password': '123456'})
recipient = call('/auth/login', 'POST', {'identifier': 'linzhou@liyu.test', 'password': '123456'})
stranger = call('/auth/login', 'POST', {'identifier': 'chenxiao@liyu.test', 'password': '123456'})
a, b, c = sender['token'], recipient['token'], stranger['token']
recipient_id = call('/me', token=b)['id']

def gift(contract):
    order = call('/orders', 'POST', {'product_id': 0, 'recipient_id': recipient_id}, a, key=str(uuid.uuid4()))
    call(f"/orders/{order['id']}/pay-test", 'POST', {}, a)
    ident = call(f"/orders/{order['id']}", token=a)['items'][0]['gift_id']
    call(f'/gifts/{ident}/puzzle', 'PUT', {'unlock_kind': 'free', 'contract_text': contract}, a)
    return ident

ident = gift('周末陪我看一场电影')
path = f'/contracts/{ident}/status'
def mark(token, value, status=200, **extra):
    return call(path, 'PUT', dict(status=value, **extra), token, status)
def own(token):
    row = next(x for x in call('/contracts', token=token) if x['gift_id'] == ident)
    detail = call(f'/gifts/{ident}', token=token)
    assert row['status'] == detail['contract_status']
    return row

for token in (a, b):
    mark(token, 'fulfilled', 404)
    assert call(f'/gifts/{ident}', token=token)['contract_status'] is None
call(f'/gifts/{ident}/open', 'POST', {}, b)
call(f'/gifts/{ident}/accept', 'POST', {'agree': True, 'recipient_name': '验证用户', 'recipient_phone': '13800138000', 'recipient_address': '验证路 1 号'}, b)
assert sql(f'SELECT count(*) FROM gift_contracts WHERE gift_id={ident}') == '0'
# Historical shared fulfillment must not masquerade as either participant's decision.
sql(f"INSERT INTO gift_contracts(gift_id,status) VALUES({ident},'fulfilled')")
assert own(a)['status'] == own(b)['status'] == 'pending'
assert own(a)['mine'] is False and own(b)['mine'] is True
mark(a, 'fulfilled')
assert own(a)['status'] == 'fulfilled' and own(b)['status'] == 'pending'
mark(b, 'fulfilled')
mark(a, 'pending')
assert own(a)['status'] == 'pending' and own(b)['status'] == 'fulfilled'
mark(a, 'pending')
assert sql(f'SELECT count(*) FROM gift_contract_marks WHERE gift_id={ident}') == '2'
with ThreadPoolExecutor(max_workers=2) as pool:
    list(pool.map(lambda entry: mark(*entry), [(a, 'fulfilled'), (b, 'pending')]))
assert own(a)['status'] == 'fulfilled' and own(b)['status'] == 'pending'
mark(None, 'fulfilled', 401)
mark(c, 'fulfilled', 404)
mark(a, 'waived', 400)
mark(a, 'fulfilled', 400, user_id=recipient_id)
call(f'/gifts/{ident}', token=c, status=404)
assert not any(x['gift_id'] == ident for x in call('/contracts', token=c))
no_contract = gift('')
call(f'/gifts/{no_contract}/open', 'POST', {}, b)
call(f'/gifts/{no_contract}/accept', 'POST', {'recipient_name': '验证用户', 'recipient_phone': '13800138000', 'recipient_address': '验证路 1 号'}, b)
call(f'/contracts/{no_contract}/status', 'PUT', {'status': 'fulfilled'}, a, 404)
assert call(f'/gifts/{no_contract}', token=b)['contract_status'] is None
assert own(a)['status'] == 'fulfilled' and own(b)['status'] == 'pending'


# Quotes are promises to the confirming user: repricing must not create an order.
body = {'product_id': 0, 'recipient_id': recipient_id}
quote = call('/orders/quote', 'POST', body, a)
price = quote['total_cents']; old_count = sql('SELECT count(*) FROM orders')
sql('UPDATE catalog SET price_cents=price_cents+100 WHERE id=0')
changed = call('/orders', 'POST', body, a, 409, key='changed-price', expected=price)
assert changed['code'] == 'quote_changed'
assert sql('SELECT count(*) FROM orders') == old_count
fresh = call('/orders/quote', 'POST', body, a)
order = call('/orders', 'POST', body, a, key='guarded-purchase', expected=fresh['total_cents'])
assert call('/orders', 'POST', dict(reversed(list(body.items()))), a, key='guarded-purchase', expected=fresh['total_cents'])['id'] == order['id']
call('/orders', 'POST', dict(body, product_id=1), a, 409, key='guarded-purchase', expected=fresh['total_cents'])
call('/orders', 'POST', body, a, 409, key='guarded-purchase', expected=fresh['total_cents']+1)
call(f"/orders/{order['id']}/pay-test", 'POST', {}, a)
call(f"/orders/{order['id']}/pay-test", 'POST', {}, a)
paid = call(f"/orders/{order['id']}", token=a)
assert paid['status']=='paid_test' and paid['total_cents']==fresh['total_cents'] and len(paid['items'])==1
box = paid['items'][0]['gift_id']
call(f'/gifts/{box}/open', 'POST', {}, b, 409)
call(f'/gifts/{box}/puzzle', 'PUT', {'unlock_kind':'question','clue':'颜色','answer':'蓝色','message':'生日快乐','contract_text':'一起看电影'}, a)
read = call(f'/gifts/{box}', token=a)
assert read['ready'] and read['unlock_kind']=='question' and read['clue']=='颜色' and read['message']=='生日快乐'
assert 'answer' not in read and 'answer_hash' not in read
hidden = call(f'/gifts/{box}', token=b)
assert hidden['sender'] is None and hidden.get('message','')==''
# Shared dates require the other participant and reject stale decisions.
schedule = f'/contracts/{ident}/schedule'
assert call(schedule, token=a)['planned_on']==call(schedule, token=b)['planned_on']==''
def propose(date, token=a):
    row=call(schedule,token=token)
    return call(schedule,'PUT',{'proposed_on':date,'expected_revision':row['revision']},token)
def decide(action, token=b, revision=None, status=200):
    if revision is None: revision=call(schedule,token=token)['revision']
    return call(schedule+'/'+action,'POST',{'expected_revision':revision},token,status)
row=propose('2028-02-29')
assert row['pending'] and row['planned_on']=='' and call(schedule,token=b)['proposed_on']=='2028-02-29'
call(f'/contracts/{ident}/calendar-export','POST',{},a,status=409)
decide('confirm',a,status=409)
decide('cancel',b,status=409)
decide('confirm',b,0,409)
call(schedule, 'PUT', {'proposed_on':'2026-02-29','expected_revision':row['revision']}, a, 400)
call(schedule, 'PUT', {'proposed_on':'2028-03-01','expected_revision':0}, a, 409)
call(schedule, token=c, status=404)
call(schedule, 'PUT', {'proposed_on':'2028-03-01','expected_revision':0}, c, 404)
decide('confirm',c,1,404)
call(schedule, token=None, status=401)
decide('confirm',b)
assert call(schedule,token=a)['planned_on']==call(schedule,token=b)['planned_on']=='2028-02-29'
propose('2028-03-02')
assert call(schedule,token=b)['planned_on']=='2028-02-29'
decide('cancel',a)
assert not call(schedule,token=b)['pending'] and call(schedule,token=b)['planned_on']=='2028-02-29'
mark(a,'pending')
assert next(x for x in call('/contracts',token=a) if x['gift_id']==ident)['planned_on']=='2028-02-29'
# Nonce lives in URL fragment; wrong key, replay, confirmed date changes and logout fail closed.
def export(token=a):
    row = call(f'/contracts/{ident}/calendar-export','POST',{},token)
    from urllib.parse import urlsplit
    link=urlsplit(row['browser_path'])
    return '/calendar-exports/'+link.path.split('/')[-1]+'/download',link.fragment
endpoint,key=export()
call(endpoint,'POST',{'key':'wrong'},status=410)
ics=call(endpoint,'POST',{'key':key})
assert ics['filename'].endswith('.ics') and 'DTSTART;VALUE=DATE:20280229\r\n' in ics['content'] and 'DTEND;VALUE=DATE:20280301\r\n' in ics['content']
import re
assert re.search(r'DTSTAMP:[0-9]{8}T[0-9]{6}Z\r\n',ics['content'])
assert 'BEGIN:VALARM' in ics['content'] and 'ATTENDEE' not in ics['content']
assert all(len(line.encode())<=75 for line in ics['content'].split('\r\n'))
call(endpoint,'POST',{'key':key},status=410)
endpoint,key=export()
propose('2028-03-01')
# Pending reschedule still exports the previously confirmed date.
assert 'DTSTART;VALUE=DATE:20280229' in call(endpoint,'POST',{'key':key})['content']
endpoint,key=export()
decide('confirm',b)
call(endpoint,'POST',{'key':key},status=409)
endpoint,key=export()
call('/auth/logout','POST',{},a,status=204)
call(endpoint,'POST',{'key':key},status=410)
mark(b,'fulfilled')
endpoint,key=export(b)
assert 'BEGIN:VALARM' not in call(endpoint,'POST',{'key':key})['content']
# Clearing the shared date also requires the other participant (a fresh session).
a=call('/auth/login','POST',{'identifier':'demo@liyu.test','password':'123456'})['token']
propose('',b)
assert call(schedule,token=b)['planned_on']=='2028-03-01'
decide('confirm',a)
assert call(schedule,token=b)['planned_on']==''
call(f'/contracts/{ident}/calendar-export','POST',{},b,status=409)
print(f'PASS: {checks} HTTP checks; quote guard, idempotency conflicts, configuration readback, mutually confirmed dates and single-use calendar exports')
