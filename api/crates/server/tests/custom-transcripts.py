#!/usr/bin/env python3
"""Compares two transcripts of custom-probe.py or custom-events.mjs against a
list of known differences.

  custom-transcripts.py <first> <this> <known-differences>

A line is told apart by its label, the text before the first ` | ` (or the whole
line when it has none). Exits 1 on a difference whose label is not in the list,
on a label in the list that no longer differs, and on transcripts of different
length. The list has one label to a line, a tab, and a word on why.
"""
import sys


def lines(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().split("\n")


def main():
    first, ours = lines(sys.argv[1]), lines(sys.argv[2])
    known = {}
    with open(sys.argv[3], encoding="utf-8") as f:
        for line in f:
            if line.strip():
                label, _, why = line.rstrip("\n").partition("\t")
                known[label] = why
    bad = 0
    if len(first) != len(ours):
        print(f"the transcripts have {len(first)} and {len(ours)} lines")
        bad += 1
    seen = set()
    for a, b in zip(first, ours):
        if a != b:
            label = a.split(" | ")[0]
            seen.add(label)
            if label not in known:
                bad += 1
                print(f"NEW DIFFERENCE\n< {a[:300]}\n> {b[:300]}")
    for label in sorted(set(known) - seen):
        bad += 1
        print(f"NO LONGER DIFFERS: {label}")
    print(f"{len(seen & set(known))} known differences, {bad} problems")
    sys.exit(1 if bad else 0)


main()
