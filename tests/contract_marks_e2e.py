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
assert 'liyu_contract_test_' in urllib.parse.urlsplit(dsn).path
checks = 0

def call(path, method='GET', data=None, token=None, status=200, key=None):
    global checks
    headers = {'Content-Type': 'application/json'}
    if token: headers['Authorization'] = 'Bearer ' + token
    if key: headers['Idempotency-Key'] = key
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

# Optional private host sessions let the UI check the same two participants.
if os.environ.get('LIYU_CONTRACT_UI_DIR'):
    root = Path(os.environ['LIYU_CONTRACT_UI_DIR'])
    for name, session in [('sender', sender), ('recipient', recipient)]:
        dest = root / name / '.host/liyu/liyu-mini/session.json'
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(json.dumps({"token": session["token"], "identifier": session["user"]["identifier"]}))
        dest.chmod(0o600)
    (root / 'gift.json').write_text(json.dumps({'id': ident}))
print(f'PASS: {checks} HTTP checks; independent/reversible marks, concurrency, access control, legacy isolation')
