#!/usr/bin/env python3
"""Run each destructive integration suite on its own disposable database."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.request
from urllib.parse import urlsplit, urlunsplit
import uuid

root = Path(__file__).resolve().parents[1]
admin_dsn = os.environ['DATABASE_URL']
parts = urlsplit(admin_dsn)
port = int(os.environ.get('LIYU_CI_PORT', '18788'))
results = []
for prefix, suite in [('browser_auth', 'browser_auth_e2e.py'),
                      ('host_auth', 'host_auth_e2e.py'),
                      ('wishlist', 'wishlist_drafts_e2e.py'),
                      ('contract', 'contract_marks_e2e.py'),
                      ('purchase', 'purchase_calendar_e2e.py'),
                      ('ai_search_test', 'catalog_search_e2e.py'),
                      ('fallback_no_token', 'test_mode_e2e.py'),
                      ('fallback_no_webhook', 'test_mode_e2e.py')]:
    name = f'liyu_{prefix}_test_{uuid.uuid4().hex[:12]}'
    subprocess.run(['psql', admin_dsn, '-v', 'ON_ERROR_STOP=1', '-c', f'CREATE DATABASE {name}'], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, DATABASE_URL=urlunsplit(parts._replace(path='/' + name)),
               LIYU_BIND=f'127.0.0.1:{port}', LIYU_TEST_DELIVERY='true', LIYU_ENV='development')
    if prefix.startswith('fallback_'):
        env.update(LIYU_TEST_DELIVERY='false', LIYU_TEST_MODE='true', LIYU_ENV='production',
                   LIYU_ADMIN_COOKIE_SECURE='true', LIYU_PUBLIC_URL='https://liyu.taidge.com',
                   LIYU_DELIVERY_WEBHOOK='https://unused-provider.invalid/send' if prefix == 'fallback_no_token' else '',
                   LIYU_DELIVERY_TOKEN='' if prefix == 'fallback_no_token' else 'unused-test-token')
    process = None
    try:
        with tempfile.TemporaryFile() as log:
            process = subprocess.Popen([str(root / 'target/debug/liyu-server')], cwd=root, env=env, stdout=log, stderr=log)
            for _ in range(60):
                if process.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/health', timeout=1) as response:
                        if response.status == 200:
                            break
                except OSError:
                    time.sleep(1)
            else:
                raise RuntimeError('server health timeout')
            if process.poll() is not None:
                log.seek(0)
                print(log.read().decode())
                raise RuntimeError('server startup failed')
            if prefix == 'browser_auth':
                subprocess.run(['cargo', 'test', '--locked'], cwd=root, env=env, check=True)
            subprocess.run(['python3', str(root / 'tests' / suite), f'http://127.0.0.1:{port}'], env=env, check=True)
            results.append({'suite': suite, 'result': 'passed', 'database': 'isolated disposable database'})
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        # The test server has exited; avoid FORCE, which can require permission
        # to terminate an unrelated autovacuum backend on the scratch database.
        for attempt in range(10):
            dropped = subprocess.run(['psql', admin_dsn, '-v', 'ON_ERROR_STOP=1', '-c', f'DROP DATABASE {name}'], capture_output=True, text=True)
            if dropped.returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise RuntimeError('Could not remove scratch database ' + name + ': ' + dropped.stderr)


output = root / 'build/verification'
output.mkdir(parents=True, exist_ok=True)
report = {'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
          'working_tree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=root, text=True).strip()),
          'unit_tests': 'cargo test --locked passed', 'api_suites': results,
          'ci_run_id': os.environ.get('GITHUB_RUN_ID'), 'scope': 'API and database tests; no GUI or real payment claim'}
(output / 'api-tests.json').write_text(json.dumps(report, indent=2) + '\n')
