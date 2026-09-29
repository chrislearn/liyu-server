"""Manage only the local liyu_dev database; configuration comes from just's dotenv."""

import os
import re
import shutil
import socket
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


def postgres_data_dir():
    if os.environ.get("PGDATA"):
        return Path(os.environ["PGDATA"]).expanduser()
    if sys.platform != "darwin" or not shutil.which("brew"):
        return None
    try:
        prefix = subprocess.check_output(["brew", "--prefix"], text=True).strip()
        version = subprocess.check_output(["pg_ctl", "--version"], text=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    match = re.search(r"PostgreSQL\) (\d+)", version)
    if not match:
        return None
    data_dir = Path(prefix) / "var" / f"postgresql@{match.group(1)}"
    return data_dir if (data_dir / "PG_VERSION").is_file() else None


def start_postgres_if_needed(url):
    port = url.port or 5432
    if socket_is_listening(url.hostname, port):
        return

    data_dir = postgres_data_dir()
    if not shutil.which("pg_ctl") or not data_dir or not (data_dir / "PG_VERSION").is_file():
        sys.exit(
            f"No PostgreSQL is listening on {url.hostname}:{port}. Start it, or set PGDATA "
            "to an existing local PostgreSQL data directory before running just dev."
        )
    result = subprocess.run(
        ["pg_ctl", "-D", str(data_dir), "-o", f"-p {port}",
         "-l", str(data_dir / "liyu-dev-postgres.log"), "-w", "start"],
        capture_output=True, text=True,
    )
    if result.returncode and not socket_is_listening(url.hostname, port):
        detail = result.stderr.strip() or result.stdout.strip()
        sys.exit(f"Could not start PostgreSQL on port {port}:\n{detail}")
    print(f"PostgreSQL ready on {url.hostname}:{port}")


def socket_is_listening(host, port):
    try:
        with socket.create_connection((host, port), timeout=1):
            return True
    except OSError:
        return False


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

    if mode == "ensure":
        start_postgres_if_needed(url)

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
