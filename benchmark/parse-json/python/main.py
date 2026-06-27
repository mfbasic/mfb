"""Reads a pre-generated JSON object with a 5000-element array plus a "tail"
field, parses it, then reads the tail back out to verify."""

import json


def main():
    with open("/tmp/mfb-bench-parse-json.json") as f:
        value = json.load(f)
    print("tail: " + json.dumps(value["tail"]))
    return 0


if __name__ == "__main__":
    main()
