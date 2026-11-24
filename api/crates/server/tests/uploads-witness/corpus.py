#!/usr/bin/env python3
"""Writes the corpus the witness is asked about: one input a line, as hex of its UTF-8.

  python3 corpus.py > corpus.hex
  dotnet run -c Release < corpus.hex > expected.txt
  UPLOADS_WITNESS_CORPUS=corpus.hex UPLOADS_WITNESS_ANSWERS=expected.txt \\
    acargo test -p stellarr-api --lib uploads::witness -- --ignored

Program.cs is the first server's own code for what a form's values and a file's
name are made of (`double.TryParse`, `int.TryParse`, `bool.TryParse`,
`Convert.TryFromBase64String`, `Path.GetFileName`, the range parser). Build it
outside the repository.
"""
import random

random.seed(7)
ATOMS = ["0", "1", "2", "5", "9", "12", "3", "00", ",", ".", "e", "E", "+", "-", " ", "\t", "\n", "\0", " ", " ",
         "Infinity", "infinity", "NaN", "nan", "true", "false", "TRUE", " true", "bytes=", "BYTES=", "=", "/", "A", "a",
         "AAAA", "AA==", "x", "..", "ż", "😀", "ſ", "ı", "İ", "\"", "'", "\\", ":", "*", "?", ";", "퟿", "�",
         "K", "1e5", "1e999", "0x", "1_0", "٣", "１", "\u0085", "\x0b", "\x0c", "\x7f", "​"]
FIXED = ["bytes=0-4", "bytes=5-", "bytes=-5", "bytes=0-0", "bytes=9223372036854775807-", "bytes=9223372036854775808-",
         "bytes= 4 - 8 ", "bytes=1,2", "bytes=2-3, 7-9", "", "A" * 44, "A" * 43 + "=", "A" * 42 + "==", "A" * 48,
         "AAAA" * 8, "AAAA" * 9, "2147483647", "2147483648", "-2147483648", "-2147483649", "  12  ", "+5", "1,000.5",
         "1,2", "1e-3", "5e-324", "1e308", "1.8e308", "  -0  ", "0.0004", "0.0005", "a" * 205 + ".png", "x/y/z.png"]
inputs = set(FIXED)
for _ in range(120000):
    inputs.add("".join(random.choice(ATOMS) for _ in range(random.randint(0, 7))))
for text in sorted(inputs):
    print(text.encode("utf-8", "surrogatepass").hex())
