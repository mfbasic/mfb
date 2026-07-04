#!/usr/bin/env python3
"""Generate the malicious .mfp for PKG-02c (operator/operand type confusion)."""
import os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import mfp_craft as m
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
MFB = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "target", "debug", "mfb")
FIXTURE = os.path.join(REPO, "tests", "security", "pkg-02c-operator-confusion")
base = m.build_base_package(HERE, MFB)
m.write_fixture_package(FIXTURE, "sec_operator_confused", m.mutate_operator_confusion(base))
