#!/usr/bin/env python3
"""A move of a channel that changes what people may do asks the instance's
owner for a second step made in the last five minutes, here and on the first
server: the same answers for a session that never passed one, one that passed
it a moment ago and one that passed it an hour ago, and for moves that change
nothing. Needs docker, sqlite3, the database and visible.json that
channels-shared-database.sh leaves in its work folder, and the oracle script.

  WORK=<that folder> ORACLE=<path of the oracle script> STELLARR_API=<binary> \
    ORACLE_ENV_FILE=<settings file> channels-second-step.py

Starts and stops its own containers (`chan2`, port 5212) and server (5291).
"""
import json, os, subprocess, sys, time, shutil, urllib.request, http.cookiejar
ROOT=os.environ["WORK"]
ORACLE=os.environ["ORACLE"]
BINARY=os.environ["STELLARR_API"]
ENV_FILE=os.environ.get("ORACLE_ENV_FILE","")
SHARED=ROOT+"/shared.db"
V=json.load(open(ROOT+"/visible.json"))
ids={c["name"]:c["id"] for c in V["ola"]["channels"]}
secs={s["name"]:s["id"] for s in V["ola"]["sections"]}
order=[c["id"] for c in V["ola"]["channels"]]
H={"X-Stellarr-Client":"web","X-Stellarr-Client-Version":"2.0.0","Content-Type":"application/json"}
def call(base,jar,method,path,body=None):
    op=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    r=urllib.request.Request(base+path,data=json.dumps(body).encode() if body is not None else None,method=method,headers=H)
    try:
        x=op.open(r); return x.status,x.read().decode()
    except urllib.error.HTTPError as e:
        return e.code,e.read().decode()
def ready(base):
    for _ in range(120):
        try:
            if urllib.request.urlopen(base+"/health/ready").status==200: return
        except Exception: pass
        time.sleep(0.5)
    raise SystemExit("not ready")
def scrub(t):
    import re
    return re.sub(r'00-[0-9a-f]{32}-[0-9a-f]{16}-00','<trace>',t)

class Oracle:
    name="stellarr-port-oracle-chan2"; base="http://127.0.0.1:5212"
    def start(self):
        subprocess.run([ORACLE,"stop","chan2"],capture_output=True)
        env=dict(os.environ,ORACLE_ENV_FILE=ENV_FILE)
        subprocess.run([ORACLE,"start","chan2","5212"],env=env,capture_output=True)
        subprocess.run(["docker","stop","-t","10",self.name],capture_output=True)
        subprocess.run(["docker","cp",SHARED,self.name+":/tmp/data/stellarr.db"],check=True)
        subprocess.run(["docker","start",self.name],capture_output=True); ready(self.base)
    def edit(self,sql):
        subprocess.run(["docker","stop","-t","10",self.name],capture_output=True)
        subprocess.run(["docker","cp",self.name+":/tmp/data/stellarr.db",ROOT+"/second-step-oracle.db"],check=True)
        subprocess.run(["sqlite3",ROOT+"/second-step-oracle.db",sql],check=True)
        subprocess.run(["docker","cp",ROOT+"/second-step-oracle.db",self.name+":/tmp/data/stellarr.db"],check=True)
        subprocess.run(["docker","start",self.name],capture_output=True); ready(self.base)
    def stop(self): subprocess.run([ORACLE,"stop","chan2"],capture_output=True)
class Rust:
    base="http://127.0.0.1:5291"; proc=None
    def launch(self):
        env=dict(os.environ,ASPNETCORE_ENVIRONMENT="Testing",Rtc__TurnSharedSecret="x",STELLARR_DATA_ROOT=ROOT+"/second-step-rust",**{"ConnectionStrings__Database":"Data Source="+ROOT+"/second-step-rust/stellarr.db"},ASPNETCORE_URLS=self.base)
        self.proc=subprocess.Popen([BINARY],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); ready(self.base)
    def start(self):
        shutil.rmtree(ROOT+"/second-step-rust",ignore_errors=True); os.makedirs(ROOT+"/second-step-rust"); shutil.copy(SHARED,ROOT+"/second-step-rust/stellarr.db"); self.launch()
    def edit(self,sql):
        self.proc.terminate(); self.proc.wait()
        subprocess.run(["sqlite3",ROOT+"/second-step-rust/stellarr.db",sql],check=True); self.launch()
    def stop(self): self.proc.terminate(); self.proc.wait()

def run(server):
    server.start()
    jar=http.cookiejar.CookieJar()
    out=[]
    st,b=call(server.base,jar,"POST","/api/auth/login",{"username":"ola","password":"Corona!2026-long"}); out.append(("login",st))
    uid=subprocess.run(["sqlite3",SHARED,"select Id from Users where Username='ola'"],capture_output=True,text=True).stdout.strip()
    server.edit(f"insert into MfaMethods (Id,UserId,Type,Name,CreatedAt,ConfirmedAt,ProtectedSecret) values ('11111111-1111-1111-1111-111111111111','{uid}','totp','Phone','2026-11-20 10:00:00','2026-11-20 10:01:00','x');")
    mv=lambda ch,sec:call(server.base,jar,"PUT","/api/admin/channels/order",{"channelIds":order,"move":{"channelId":ids[ch],"sectionId":secs[sec] if sec else None}})
    out.append(("stale, permission-changing (general into Bravo)",)+mv("general","Bravo"))
    out.append(("stale, harmless (news into Alpha)",)+mv("news","Alpha"))
    out.append(("stale, plain reorder",)+call(server.base,jar,"PUT","/api/admin/channels/order",{"channelIds":order[::-1]}))
    server.edit(f"update UserSessions set MfaCompletedAt=strftime('%Y-%m-%d %H:%M:%f','now') where UserId='{uid}';")
    out.append(("fresh, permission-changing",)+mv("general","Bravo"))
    server.edit(f"update UserSessions set MfaCompletedAt=strftime('%Y-%m-%d %H:%M:%f','now','-1 hour') where UserId='{uid}';")
    out.append(("an hour old, permission-changing (out of Bravo)",)+mv("secret",None))
    server.stop()
    return [tuple(scrub(x) if isinstance(x,str) else x for x in o) for o in out]
a=run(Oracle()); b=run(Rust())
for x,y in zip(a,b): print("same     " if x==y else "DIFFERENT", x[0], x[1], x[2:] if x!=y else "", y[1:] if x!=y else "")
