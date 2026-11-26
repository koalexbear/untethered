"""What a debug Untethered client synced: its QA event log (rust-client/src/engine/qa.rs).

The fixtures read this where they read the client's gomuks database before.
Desktop clients append one JSON object per line to the file
UNTETHERED_QA_EVENTS names; use_profile() keeps that file in the profile
folder. In the browser the engine's worker posts each entry as a
{"Qa": entry} message; WORKER_HOOK collects them in window.qaEvents.

Entries:
  {"kind":"event", room_id, event_id, sender, type, content, encrypted,
   notify, highlight, sound[, redecrypted]}
     type/content decrypted where the device could decrypt the event;
     notify/highlight/sound: the SDK's push rule evaluation (gomuks's
     unread_type flags). A key that came late gives a second entry with
     "redecrypted": true.
  {"kind":"alert", room_id, event_id, sender, body, sound}
     the notifications the engine passed to the app.
  {"kind":"account_data", type, content}
"""
import json
from pathlib import Path


def log(profile):
    return Path(profile)/"qa-events.jsonl"


def use_profile(env, profile):
    """Run a client on `profile`, logging to its QA event log."""
    env["UNTETHERED_PROFILE"] = str(profile)
    env["UNTETHERED_QA_EVENTS"] = str(log(profile))
    return env


def entries(profile, kind="event"):
    try:
        text = log(profile).read_text()
    except FileNotFoundError:
        return []
    found = []
    for line in text.splitlines():
        try:
            entry = json.loads(line)
        except ValueError:
            continue  # the client is still writing this line
        if entry.get("kind") == kind:
            found.append(entry)
    return found


def decrypted(entry):
    return entry["type"] != "m.room.encrypted"


def event(profile, room, event_id):
    """The event as the device first read it decrypted, or None."""
    return find(entries(profile), room, event_id)


def find(events, room, event_id):
    return next((e for e in events if e["room_id"] == room and e["event_id"] == event_id and decrypted(e)), None)


def account_data(profile, kind):
    """The latest content of a global account data event the device synced."""
    found = [e["content"] for e in entries(profile, "account_data") if e["type"] == kind]
    return found[-1] if found else None


# Collect the browser engine's QA entries (engine/worker.rs) without touching
# anything the page does with its worker. Each entry also records whether the
# tab was hidden when the worker reported it.
WORKER_HOOK = """window.qaEvents=[];
  window.Worker=class extends Worker{constructor(...args){super(...args);
    this.addEventListener('message',({data})=>{
      if(typeof data?.json!=='string' || !data.json.startsWith('{"Qa"'))return;
      const entry=JSON.parse(data.json).Qa;
      entry.hidden=document.hidden;
      window.qaEvents.push(entry);
    });
  }}"""


def web_event(page, room, event_id):
    """Like event(), from a page that ran WORKER_HOOK."""
    return page.evaluate("""([r,id])=>(window.qaEvents||[]).find(e=>e.kind==='event' && e.room_id===r &&
        e.event_id===id && e.type!=='m.room.encrypted') ?? null""", [room, event_id])


def web_account_data(page, kind):
    return page.evaluate("""k=>(window.qaEvents||[]).filter(e=>e.kind==='account_data' && e.type===k)
        .map(e=>e.content).pop() ?? null""", kind)
