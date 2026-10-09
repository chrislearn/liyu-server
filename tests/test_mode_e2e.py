"""Fixed-code fallback on an isolated production-mode database, without a provider."""
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
import uuid

base = sys.argv[1]

def call(path, data, token=None, status=200, method='POST'):
    headers = {'Content-Type': 'application/json'}
    if token: headers['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request(base + '/api/v1' + path, json.dumps(data).encode(), headers, method=method)
    try:
        response = urllib.request.urlopen(request)
    except urllib.error.HTTPError as error:
        response = error
    assert response.status == status, (path, response.status, status)
    return json.loads(response.read())

email = 'fixed-' + uuid.uuid4().hex + '@example.test'
challenge = call('/auth/challenges', {'kind': 'email', 'value': email, 'purpose': 'register'})
assert challenge['test_code'] == '123456'
registration = {'identifier': email, 'password': 'test-password-2026', 'challenge_id': challenge['challenge_id']}
call('/auth/register', {**registration, 'code': '000000'}, status=400)
account = call('/auth/register', {**registration, 'code': '123456'})
assert account['test_delivery'] is True
phone = '13800138000'
challenge = call('/auth/challenges', {'kind': 'phone', 'value': phone, 'purpose': 'bind'}, account['token'])
assert challenge['test_code'] == '123456'
body = {'value': phone, 'challenge_id': challenge['challenge_id'], 'code': '123456'}
call('/me/phone', body, account['token'], method='PUT')
call('/me/phone', body, account['token'], status=400, method='PUT')
count = subprocess.check_output(['psql', os.environ['DATABASE_URL'], '-Atc', "SELECT count(*) FROM delivery_outbox WHERE event_key LIKE 'challenge:%'"], text=True).strip()
assert count == '0', 'Fixed-code fallback must not queue verification messages'
print('Fixed-code fallback: registration, binding, wrong code, replay and no delivery passed')
