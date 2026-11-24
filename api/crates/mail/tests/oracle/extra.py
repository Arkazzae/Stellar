"""How extra.json was made, on the same server as en.json after it: a second person with a name full of characters the HTML must escape; the
mail for their password reset is saved to captures/extra.json."""
import json, sys, time
sys.argv = ["x", "en"]
exec(open("scenario.py").read().split("mailpit(\"/api/v1/messages\", method=\"DELETE\")")[0])
NAME = "Zoë Ünï \U0001F600 <&>'\" ÿĀ ©"
mailpit("/api/v1/messages", method="DELETE")
sam = Client()
need(sam.call("POST", "/api/auth/login", {"username": "sam", "password": "sams horse battery 4D!"}), "login sam")
need(sam.call("PATCH", "/api/me", {"displayName": NAME}), "rename")
need(sam.call("POST", "/api/auth/password/forgot", {"email": "sam@example.test"}), "forgot")
wait_for(1)
m = mailpit("/api/v1/messages")["messages"][0]
d = mailpit("/api/v1/message/" + m["ID"])
json.dump({"language": "en", "display_name": NAME, "events": {}, "mails": [{
    "subject": d["Subject"], "text": d["Text"], "html": d["HTML"], "from": d["From"], "to": d["To"],
    "message_id": d["MessageID"], "images": [{"content_id": p["ContentID"], "file_name": p["FileName"],
    "content_type": p["ContentType"], "size": p["Size"]} for p in d.get("Inline", [])]}]},
    open("captures/extra.json", "w"), ensure_ascii=False, indent=1)
print(d["Subject"]); print(repr(d["Text"][:120]))
