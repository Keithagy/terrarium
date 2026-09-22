import os
from celery import Celery

celery = Celery(broker=os.environ.get("REDIS_URL"))


def enqueue_welcome(user):
    send_welcome.delay(user["name"])


@celery.task
def send_welcome(name):
    print("welcome", name)
