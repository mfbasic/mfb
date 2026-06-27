"""Reads a pre-generated string of 200 space-separated numbers, then uses a
regex to locate every run of digits and counts the matches."""

import re


def main():
    with open("/tmp/mfb-bench-parse-regex.txt") as f:
        text = f.read()

    matches = re.findall(r"[0-9]+", text)
    print("matches: " + str(len(matches)))
    return 0


if __name__ == "__main__":
    main()
