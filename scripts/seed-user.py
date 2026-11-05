#!/usr/bin/env python3
"""Bind an API token to a real user/team in a fresh smoke-test database.

Usage: seed-user.py <data-dir> <email> <name> <token> [team-name]
The server must have created dok.db already (wait for /api/v1/health first).
"""
import hashlib
import os
import sqlite3
import sys
import time

data, email, name, token = sys.argv[1:5]
team_name = sys.argv[5] if len(sys.argv) > 5 else "Smoke Team"
slug = team_name.lower().replace(" ", "-")
user_id = "smoke-user"
team_id = "smoke-team"
now = int(time.time())

con = sqlite3.connect(os.path.join(data, "dok.db"))
cur = con.cursor()
cur.execute(
    "INSERT OR IGNORE INTO users (id, email, name, created_at) VALUES (?,?,?,?)",
    (user_id, email.lower(), name, now),
)
cur.execute(
    "INSERT OR IGNORE INTO teams (id, name, slug, created_at) VALUES (?,?,?,?)",
    (team_id, team_name, slug, now),
)
cur.execute(
    "INSERT OR REPLACE INTO members (team_id, user_id, role, created_at) VALUES (?,?,?,?)",
    (team_id, user_id, "owner", now),
)
h = hashlib.sha256(token.encode()).hexdigest()
cur.execute("UPDATE tokens SET user_id = ?, name = 'smoke' WHERE hash = ?", (user_id, h))
if cur.rowcount == 0:
    cur.execute(
        "INSERT INTO tokens (hash, name, created_at, user_id) VALUES (?,?,?,?)",
        (h, "smoke", now, user_id),
    )
con.commit()
con.close()
print("seeded smoke user", email)
