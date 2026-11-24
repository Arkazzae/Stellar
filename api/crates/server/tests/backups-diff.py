"""Compares two transcripts of backups-oracle.mjs section by section, naming each
section that differs: first server (<), this server (>). Exits 1 on a difference."""
import sys


def sections(path):
    found, label = {}, None
    order = []
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.startswith("## "):
            label = line[3:]
            while label in found:
                label += "'"
            found[label] = []
            order.append(label)
        elif label is not None:
            found[label].append(line)
    return found, order


first, order = sections(sys.argv[1])
this, order_this = sections(sys.argv[2])
differs = 0
for label in order + [x for x in order_this if x not in first]:
    a, b = first.get(label), this.get(label)
    if a != b:
        differs += 1
        print(f"## {label}")
        print("<", "\n< ".join(a) if a is not None else "(missing)")
        print(">", "\n> ".join(b) if b is not None else "(missing)")
sys.exit(1 if differs else 0)
