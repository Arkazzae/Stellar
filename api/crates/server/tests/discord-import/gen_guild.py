#!/usr/bin/env python3
"""A generated Discord server as the bot reads it: Discord's own JSON, one
object with the answers to every read the importer makes. Deterministic for a
seed, so two writers can be given the same one.

    gen_guild.py OUT.json [--messages N] [--seed S] [--hostile]
"""
import argparse
import json
import random

GUILD = "41771983423143936"
BASE = 1_100_000_000_000_000_000  # message snowflakes count up from here


def at(minute):
    total = minute * 60
    days, rest = divmod(total, 86400)
    # 2024 is a leap year; messages stay inside it
    months = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    month = 0
    while days >= months[month]:
        days -= months[month]
        month += 1
    return "2024-%02d-%02dT%02d:%02d:%02d+00:00" % (month + 1, days + 1, rest // 3600, rest % 3600 // 60, rest % 60)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("--messages", type=int, default=3000)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--hostile", action="store_true")
    args = parser.parse_args()
    rng = random.Random(args.seed)

    roles = [
        {"id": GUILD, "name": "@everyone", "position": 0, "permissions": "1024"},
        {"id": "5", "name": "Mods", "position": 1, "permissions": "0"},
        {"id": "6", "name": "@Dev team", "position": 2, "permissions": "0"},
    ]
    channels = [
        {"id": "900", "type": 4, "name": "Talk", "position": 1},
        {"id": "800", "type": 4, "name": "Games & Fun", "position": 0},
        {"id": "700", "type": 4, "name": "   ", "position": 2},
    ]

    def channel(id, name, kind=0, parent=None, position=0, topic=None, slow=0, limit=0, hidden=False):
        value = {
            "id": id, "type": kind, "name": name, "position": position, "parent_id": parent,
            "topic": topic, "rate_limit_per_user": slow, "user_limit": limit,
        }
        if hidden:
            value["permission_overwrites"] = [{"id": GUILD, "type": 0, "allow": "0", "deny": "1024"}]
        channels.append(value)

    channel("100", "general", parent="900")
    channel("110", "General", parent="900", topic="  Hello  ", slow=30)
    channel("120", "Lounge", 2, parent="900", limit=5)
    channel("130", "help", 15, parent="800", topic="t" * 1500)
    channel("140", "Town Hall", 13, parent="800", limit=10000)
    channel("150", "\U0001F4E2\u30FBnews", 5)
    channel("160", "staff", parent="900", position=3, hidden=True)
    channel("170", "gallery", 16, parent="800")
    channel("180", "\u017c\u00f3\u0142\u0107 2 ?!", parent="800", position=2)
    channel("190", "x" * 120, parent="700")
    text_channels = ["100", "110", "130", "150", "160", "170", "180", "190"]
    voice_channels = ["120"]

    members = []
    member_ids = []
    for index in range(40):
        id = str(2000 + index)
        member_ids.append(id)
        members.append({
            "user": {"id": id, "username": "user%d" % index, "global_name": ("Global %d" % index) if index % 3 == 0 else None},
            "nick": ("Nick %d" % index) if index % 4 == 0 else None,
            "roles": [], "joined_at": "2023-01-01T00:00:00+00:00",
        })

    messages = {c["id"]: [] for c in channels if c["type"] != 4}
    threads = []
    pins = {}
    counter = [0]

    def next_id():
        counter[0] += 1
        return str(BASE + counter[0] * 1000 + rng.randrange(0, 999))

    def message(chan, content, minute, author=None, kind=0, reply=None, pinned=False, edited=None,
                webhook=None, attachments=0, sticker=False, embed=False, forwarded=None, id=None, username=None):
        id = id or next_id()
        author = author or rng.choice(member_ids)
        value = {
            "id": id, "channel_id": chan, "type": 19 if reply else kind, "content": content,
            "timestamp": at(minute), "edited_timestamp": at(edited) if edited else None, "pinned": pinned,
            "webhook_id": webhook,
            "author": {"id": webhook or author, "username": username or "user%s" % author[-2:], "global_name": None,
                       "bot": bool(webhook)},
            "attachments": [{"id": "%s%d" % (id, i), "filename": "photo%d.png" % i, "size": 1024,
                             "url": "https://cdn.discordapp.com/attachments/%s/%s%d/photo%d.png" % (chan, id, i, i),
                             "content_type": "image/png"} for i in range(attachments)],
            "embeds": [{"type": "rich", "title": "Status", "description": "All good"}] if embed else [],
            "sticker_items": [{"id": "749054660769218631", "name": "Wave", "format_type": 1}] if sticker else [],
        }
        if reply:
            value["message_reference"] = {"type": 0, "message_id": reply, "channel_id": chan}
        elif forwarded is not None:
            value["message_reference"] = {"type": 1, "message_id": "1", "channel_id": chan}
            value["message_snapshots"] = [{"message": {"content": forwarded, "attachments": []}}]
        messages[chan].append(value)
        if pinned:
            pins.setdefault(chan, []).append((at(minute + 1), value))
        return id

    words = ["alpha", "beta", "gamma", "quokka", "migration", "friday", "build", "deploy", "coffee", "\u017c\u00f3\u0142w", "\U0001F600"]

    def text():
        parts = [rng.choice(words) for _ in range(rng.randrange(1, 14))]
        roll = rng.random()
        if roll < 0.10:
            parts.insert(rng.randrange(len(parts) + 1), "<@%s>" % rng.choice(member_ids))
        elif roll < 0.14:
            parts.insert(rng.randrange(len(parts) + 1), "<@!%s>" % rng.choice(member_ids))
        elif roll < 0.18:
            parts.insert(rng.randrange(len(parts) + 1), "<#%s>" % rng.choice(text_channels))
        elif roll < 0.22:
            parts.insert(rng.randrange(len(parts) + 1), "<@&%s>" % rng.choice(["5", "6", "77"]))
        elif roll < 0.24:
            parts.insert(rng.randrange(len(parts) + 1), "@everyone")
        elif roll < 0.27:
            parts.insert(0, "**bold** _it_ `code` ||spoiler||")
        elif roll < 0.29:
            parts.append("https://example.com/%d" % rng.randrange(1000))
        elif roll < 0.30:
            parts.append("<:blob:123456789012345678> <a:dance:987654321098765432>")
        elif roll < 0.31:
            parts.append("<@999999999999999>")
        return " ".join(parts)

    per_channel = max(1, args.messages // len(text_channels))
    minute = 1
    for chan in text_channels:
        ids = []
        for n in range(per_channel):
            minute += rng.randrange(0, 3)
            roll = rng.random()
            if roll < 0.02:
                message(chan, "", minute, kind=rng.choice([7, 6, 8, 3, 4]), author=rng.choice(member_ids))
            elif roll < 0.03:
                message(chan, "", minute, embed=True)
            elif roll < 0.04:
                message(chan, "", minute, sticker=True)
            elif roll < 0.06:
                message(chan, "", minute, attachments=rng.randrange(1, 3))
            elif roll < 0.07:
                message(chan, text(), minute, attachments=1)
            elif roll < 0.08:
                message(chan, "", minute, forwarded="forwarded words")
            elif roll < 0.09:
                message(chan, "   ", minute)
            elif roll < 0.11:
                ids.append(message(chan, text(), minute, webhook="7%d" % rng.randrange(3), username="Hook %d" % rng.randrange(9)))
            elif roll < 0.30 and ids:
                target = rng.choice(ids[-30:]) if rng.random() < 0.9 else str(BASE - 5)
                ids.append(message(chan, text(), minute, reply=target, edited=minute + 2 if rng.random() < 0.2 else None))
            else:
                ids.append(message(chan, text(), minute, pinned=rng.random() < 0.01,
                                   edited=minute + 2 if rng.random() < 0.1 else None))
        # the chat of the first public thread's starter
        if chan == "100" and ids:
            starter = ids[len(ids) // 2]
            threads.append({"id": starter, "type": 11, "parent_id": chan, "name": "side talk",
                            "thread_metadata": {"archived": False, "locked": False}})
            messages[starter] = []
            for n in range(25):
                minute += 1
                message(starter, text(), minute)
            # a thread whose starter was deleted
            gone = str(BASE - 99)
            threads.append({"id": gone, "type": 11, "parent_id": chan, "name": "orphan **talk**",
                            "thread_metadata": {"archived": False, "locked": False}})
            messages[gone] = []
            for n in range(5):
                minute += 1
                message(gone, text(), minute)
            threads.append({"id": "3001", "type": 12, "parent_id": chan, "name": "private",
                            "thread_metadata": {"archived": False, "locked": False}})
            messages["3001"] = []
            for n in range(6):
                minute += 1
                message("3001", text(), minute)
    # forum posts: the opening message has the post's id
    for post in range(6):
        id = next_id()
        threads.append({"id": id, "type": 11, "parent_id": "130", "name": "@jo **post %d** <@2001>" % post,
                        "thread_metadata": {"archived": False, "locked": False}})
        messages[id] = []
        minute += 1
        message(id, "how do I <@2001>?", minute, id=id)
        for n in range(rng.randrange(1, 8)):
            minute += 1
            message(id, text(), minute)
    for chan in voice_channels:
        for n in range(3):
            minute += 1
            message(chan, "said in voice", minute)

    if args.hostile:
        message("100", "nul\u0000 esc\u001b bidi \u202etxt \ufeffend <@2001>\u0000", minute + 1, username="a\u0000b" + "n" * 5000)
        message("100", "x" * 20001, minute + 2)
        message("100", "\U0001F600" * 10000, minute + 3)
        message("100", "with files", minute + 4)
        messages["100"][-1]["attachments"] = [{
            "id": "55", "filename": "../../etc/passwd", "size": 2 ** 62,
            "url": "https://cdn.discordapp.com/attachments/1/2/..%2f..%2fx", "content_type": "text/html\r\nX: 1"}]
        message("100", "", minute + 5, attachments=1)
        messages["100"][-1]["attachments"][0]["filename"] = "C:\\windows\\system32\\x.exe"
        messages["100"][-1]["attachments"][0]["size"] = -1
        message("100", "<@2002> <@&5> <#150> \\<@2003> <@2004" , minute + 6, author="2003", username="")

    guild = {"channels": channels, "roles": roles, "members": members, "threads": threads,
             "messages": messages, "pins": {k: v for k, v in pins.items()}}
    with open(args.out, "w") as handle:
        json.dump(guild, handle)
    total = sum(len(v) for v in messages.values())
    print("messages", total, "channels", len(channels), "threads", len(threads))


main()
