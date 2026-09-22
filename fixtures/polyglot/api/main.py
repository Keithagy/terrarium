import os
from fastapi import FastAPI
from services.users import list_users, create_user
from services import billing

app = FastAPI()
DATABASE_URL = os.environ["DATABASE_URL"]


@app.get("/api/users")
def get_users():
    return list_users()


@app.post("/api/users")
def post_user(payload: dict):
    user = create_user(payload)
    billing.enqueue_welcome(user)
    return user


@app.get("/api/users/{user_id}")
def get_user(user_id: int):
    return list_users(only=user_id)
