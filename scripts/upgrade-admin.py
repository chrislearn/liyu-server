#!/usr/bin/env python3
"""Apply only the consolidated migration's admin section to an existing DB.
Run via `just upgrade-admin` so .env is loaded. Does not reset any user data.
"""
import os
import pathlib
import subprocess
import sys

url = os.environ.get('DATABASE_URL')
if not url:
    sys.exit('DATABASE_URL is required; use just upgrade-admin')

def psql(sql, *flags):
    result = subprocess.run(['psql', '-X', '-v', 'ON_ERROR_STOP=1', *flags, url],
                            input=sql, text=True, capture_output=True)
    if result.returncode:
        sys.exit(result.stderr)
    return result.stdout.strip()

state = psql("SELECT (to_regclass('public.administrators') IS NOT NULL)::int + "
             "(to_regclass('public.admin_sessions') IS NOT NULL)::int + "
             "(to_regclass('public.catalog_id_seq') IS NOT NULL)::int", '-At')
if state == '3':
    print('Admin schema is already installed; no changes made.')
elif state == '0':
    migration = pathlib.Path(__file__).resolve().parents[1] / 'migrations/20260926000000_init/up.sql'
    section = migration.read_text().split('-- 12. web administration', 1)[1].split('-- 13. management domains',1)[0]
    psql('-- 12. web administration' + section, '-1')
    print('Admin tables and product ID sequence installed. Existing users, catalog and sessions preserved.')
else:
    sys.exit('Partial admin schema detected. Stop and inspect it before upgrading.')

# The extension uses the same migration file; serialize with server startup.
migration = pathlib.Path(__file__).resolve().parents[1] / 'migrations/20260926000000_init/up.sql'
section = migration.read_text().split('-- 13. management domains', 1)[1]
psql("BEGIN; SELECT pg_advisory_xact_lock(731129927); DO $upgrade$ BEGIN "
     "IF to_regclass('management_schema_version') IS NULL THEN EXECUTE $domain$"
     + section + "$domain$; END IF; END $upgrade$; COMMIT;")
print('Management schema ready; existing business data preserved.')
