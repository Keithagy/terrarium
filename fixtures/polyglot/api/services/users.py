import sqlite3


def connect():
    return sqlite3.connect("users.db")


def list_users(only=None):
    conn = connect()
    cursor = conn.cursor()
    cursor.execute("SELECT * FROM users")
    return cursor.fetchall()


def create_user(payload):
    conn = connect()
    conn.execute("INSERT INTO users VALUES (?)", (payload["name"],))
    return payload
