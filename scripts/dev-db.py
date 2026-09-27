"""Manage only the local liyu_dev database; configuration comes from just's dotenv."""

import os
import subprocess
import sys
from urllib.parse import urlsplit, urlunsplit


def main():
    mode = sys.argv[1] if len(sys.argv) == 2 else ""
    if mode not in {"ensure", "reset"}:
        sys.exit("Usage: dev-db.py ensure|reset")
    url = urlsplit(os.environ.get("DATABASE_URL", ""))
    if (
        url.scheme not in {"postgres", "postgresql"}
        or url.hostname not in {"localhost", "127.0.0.1", "::1"}
        or url.path != "/liyu_dev"
        or url.query
        or url.fragment
    ):
        sys.exit("Database management requires a local DATABASE_URL ending in /liyu_dev.")
    # Connect to postgres because the target database may not exist yet.
    admin_url = urlunsplit(url._replace(path="/postgres"))
    env = os.environ.copy()
    env["PGCONNECT_TIMEOUT"] = "5"

    def sql(statement):
        return subprocess.run(
            ["psql", "--dbname", admin_url, "-X", "--no-password", "--set", "ON_ERROR_STOP=1", "-At", "-c", statement],
            env=env, check=True, capture_output=True, text=True,
        ).stdout.strip()

    try:
        if mode == "reset":
            sql('DROP DATABASE IF EXISTS liyu_dev WITH (FORCE)')
        if sql("SELECT 1 FROM pg_database WHERE datname = 'liyu_dev'") != "1":
            sql("CREATE DATABASE liyu_dev")
        print("liyu_dev ready" if mode == "ensure" else "liyu_dev recreated; run just dev to migrate and seed.")
    except FileNotFoundError:
        sys.exit("psql is required; install PostgreSQL client tools and add them to PATH.")
    except subprocess.CalledProcessError as error:
        sys.exit(error.stderr.strip())


if __name__ == "__main__":
    main()
