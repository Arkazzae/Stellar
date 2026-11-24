#!/usr/bin/env python3
"""Writes the expected values the zone tests compare against.

It copies a fixed list of zone files out of the system's database into
tests/zoneinfo/ (so the tests do not depend on the tzdata of the machine they
run on) and asks Python's zoneinfo, an implementation independent of the
crate, for the offset and abbreviation of each zone:

  T  every transition found between 1970 and 2100: the zone, the first
     second the new rules apply, and what applied just before and from then on
  S  seeded random moments between 1850 and 2100

Run it from anywhere:  python3 -I generate_expected.py [system zoneinfo dir]
"""
import datetime
import os
import random
import shutil
import sys
import zoneinfo

ZONES = """
UTC Etc/GMT+5 Etc/GMT-14
Europe/Warsaw Europe/London Europe/Dublin Europe/Moscow Europe/Istanbul Europe/Kyiv Europe/Lisbon
America/New_York America/Los_Angeles America/Sao_Paulo America/Santiago America/St_Johns
America/Caracas America/Havana America/Mexico_City America/Asuncion
Australia/Sydney Australia/Lord_Howe Australia/Adelaide Australia/Perth
Asia/Kolkata Asia/Kathmandu Asia/Tokyo Asia/Tehran Asia/Gaza Asia/Pyongyang Asia/Shanghai
Pacific/Chatham Pacific/Apia Pacific/Kiritimati Pacific/Auckland Pacific/Honolulu
Africa/Cairo Africa/Casablanca Africa/Johannesburg Africa/Monrovia
Antarctica/Troll Atlantic/Azores Indian/Maldives
""".split()

here = os.path.dirname(os.path.abspath(__file__))
source = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/zoneinfo"
target = os.path.join(here, "zoneinfo")
shutil.rmtree(target, ignore_errors=True)
for name in ZONES:
    destination = os.path.join(target, name)
    os.makedirs(os.path.dirname(destination), exist_ok=True)
    shutil.copyfile(os.path.join(source, name), destination)  # follows links

zoneinfo.reset_tzpath(to=[target])
epoch = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)


def state(zone, moment):
    local = (epoch + datetime.timedelta(seconds=moment)).astimezone(zone)
    return int(local.utcoffset().total_seconds()), local.tzname()


def seconds(year, month=1, day=1):
    return int((datetime.datetime(year, month, day, tzinfo=datetime.timezone.utc) - epoch).total_seconds())


start, end, step = seconds(1970), seconds(2100), 12 * 3600
rng = random.Random(8536)
lines = []
for name in ZONES:
    zone = zoneinfo.ZoneInfo(name)
    key = name
    moment, last = start, state(zone, start)
    while moment < end:
        following = moment + step
        now = state(zone, following)
        if now != last:
            low, high = moment, following  # state(low) == last, state(high) != last
            while high - low > 1:
                middle = (low + high) // 2
                if state(zone, middle) == last:
                    low = middle
                else:
                    high = middle
            before, after = state(zone, high - 1), state(zone, high)
            lines.append(f"T\t{key}\t{high}\t{before[0]}\t{before[1]}\t{after[0]}\t{after[1]}")
            last = now
        moment = following
    for _ in range(150):
        point = rng.randrange(seconds(1850), seconds(2100))
        offset, abbreviation = state(zone, point)
        lines.append(f"S\t{key}\t{point}\t{offset}\t{abbreviation}")

with open(os.path.join(here, "expected.tsv"), "w", encoding="utf-8") as out:
    out.write(f"# zoneinfo {os.popen('cat ' + source + '/+VERSION 2>/dev/null').read().strip()}; python {sys.version.split()[0]}\n")
    out.write("\n".join(lines) + "\n")
print(len(lines), "lines")
