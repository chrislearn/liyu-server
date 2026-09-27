# coding: utf-8
"""Real-mode verification over a certificate-validated local HTTPS provider.
Requires DATABASE_URL naming a disposable contact_test database and a built
target/debug/liyu-server. Generates no external email/SMS, cleans its processes.
"""
import http.server
import json
import os
import pathlib
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

assert 'contact_test' in urllib.parse.urlsplit(os.environ['DATABASE_URL']).path
messages=[]
class Bridge(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        assert self.headers.get('Authorization')=='Bearer local-provider-test'
        messages.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        self.send_response(200);self.end_headers()
    def log_message(self,*args):pass

root=pathlib.Path(tempfile.mkdtemp(prefix='liyu-provider-'))
key=root/'key.pem';cert=root/'cert.pem'
subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(key),'-out',str(cert),'-days','1','-subj','/CN=localhost','-addext','subjectAltName=DNS:localhost,IP:127.0.0.1'],check=True,capture_output=True)
key.chmod(0o600)
provider=http.server.HTTPServer(('127.0.0.1',18822),Bridge)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(cert,key)
provider.socket=context.wrap_socket(provider.socket,server_side=True)
threading.Thread(target=provider.serve_forever,daemon=True).start()
env={**os.environ,'LIYU_TEST_DELIVERY':'false','LIYU_BIND':'127.0.0.1:18821','LIYU_PUBLIC_URL':'https://localhost:18821','LIYU_DELIVERY_WEBHOOK':'https://localhost:18822/send','LIYU_DELIVERY_TOKEN':'local-provider-test','CURL_CA_BUNDLE':str(cert),'LIYU_DATA_DIR':str(root/'media'),'LIYU_ADMIN_PASSWORD':'contact_test_admin_20260928'}
process=subprocess.Popen(['target/debug/liyu-server'],env=env,stdout=open(root/'server.log','w'),stderr=subprocess.STDOUT)
base='http://127.0.0.1:18821'
def call(path,body,status=200):
    r=urllib.request.Request(base+path,headers={'Content-Type':'application/json'},data=json.dumps(body).encode())
    try:response=urllib.request.urlopen(r,timeout=10)
    except urllib.error.HTTPError as e:response=e
    value=json.load(response);assert response.status==status,(path,response.status,value)
    return value
try:
    deadline=time.time()+10
    while time.time()<deadline:
        try:urllib.request.urlopen(base+'/health',timeout=1);break
        except urllib.error.URLError:time.sleep(.1)
    assert process.poll() is None
    contact='real-mode-'+uuid.uuid4().hex+'@example.test'
    c=call('/api/v1/auth/challenges',dict(kind='email',value=contact,purpose='register'))
    assert c['test_code'] is None
    deadline=time.time()+15
    while time.time()<deadline and not any(m['destination']==contact for m in messages):time.sleep(.1)
    m=next(m for m in messages if m['destination']==contact)
    assert m['payload']['type']=='verification' and m['payload']['code'].isdigit() and len(m['payload']['code'])==6
    body=dict(identifier=contact,password='real-mode-password',challenge_id=c['challenge_id'],code='not-a-code')
    call('/api/v1/auth/register',body,status=400)
    body['code']=m['payload']['code']
    account=call('/api/v1/auth/register',body)
    assert account['test_delivery'] is False
    call('/api/v1/auth/register',body,status=400)
    print('real-mode HTTPS verification: provider auth, TLS, random code, registration and replay checks passed')
finally:
    process.terminate();process.wait(timeout=10)
    provider.shutdown()
