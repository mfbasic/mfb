okay, I want to start adding the generic functions to the compiler now. read "## 12. Built-in JSON Package" in the spec.

I'm looking for a minimum 2 tests for each function a func-json-*-valid and func-json-*-invalid. 
Valid test should test all usage patterns and overloads for the function, edge-cases included.

include json-read-valid, json-read-invalid, json-write tests as well that read/write actual JSON files.

If you need more thant 2 test files, add them.

as there is not system standard library, this mst be written in asm.

for ArchLinux aarch64 glibc: ssh -p 2222 test@127.0.0.1
for Kali aarch64 glibc: ssh -p 2223 test@127.0.0.1
for Apline aarch64 musl: ssh -p 2224 test@127.0.0.1

1) Review the spec "## 12. Built-in JSON Package"
2) Implement or verify a single function including all it's overloads
3) Add or verify the test files (valid and invalid)
4) Add or Verify src/man/builtins/json/<func>.txt is correct
5) Run the `mfb man` to view the help file
6) Run all tests
7) goto #1 until all "## 12. Built-in JSON Package" are implemented

Do not stop after each function, continue until complete
Do not stop at a blocker, skip over it unti all other non-blocked functions are complete.
Missing native helpers or lowered code is **not** a blocker, it is part of the task.
Missing Linker support is **not** a blocker, add it.

---

adduser test
passwd test
sed -i '/^PasswordAuthentication/d;/^PermitEmptyPasswords/d;/^UsePAM/d;/^PermitRootLogin/d' /etc/ssh/sshd_config

cat >> /etc/ssh/sshd_config <<'EOF'
PasswordAuthentication yes
PermitEmptyPasswords no
UsePAM no
PermitRootLogin yes
EOF

/usr/sbin/sshd -t
rc-service sshd restart

---

REVIEW AND VERIFICATION TASK

OBJECTIVE:
Verify correctness and document inconsistencies across manual pages,
specifications, source code, and tests.

PHASE 0: VALIDATE EXISTING FIXME ITEMS
Repeat until all FIXME items have been reviewed:

1. Find the next unreviewed FIXME item in manual pages or
   specifications/review_list.txt
2. Verify the inconsistency still exists by:
   - Reading the relevant manual page
   - Checking specifications/*
   - Reviewing src/**
   - Reviewing tests/**
3. If the inconsistency no longer exists:
   - Remove the FIXME section or item
4. If the inconsistency still exists:
   - Mark as reviewed (add timestamp or note)
5. Move to the next FIXME item

PHASE 1: MANDATORY ITEM-BY-ITEM REVIEW
Process EVERY item in specifications/review_list.txt marked with [TODO].
Do NOT skip items. Do NOT scan repository instead.
Do NOT move to Phase 2 until ALL items are marked [DONE].

For each item:
1. Select the first item not marked [DONE]
2. Read the corresponding manual page
3. Read all relevant specifications/*
4. Review all related src/**
5. Review all related tests/**
6. Document inconsistencies (if any)
7. Mark the item [DONE]
8. Repeat until zero [TODO] items remain

PHASE 2: COMPREHENSIVE SPECIFICATION REVIEW
Repeat until 3 consecutive reviews find zero new FIXME items:

9. Read all specifications/*
10. Review all src/**
11. Append "FIXME" section to specifications/review_list.txt containing:
    * Any specification requirement not implemented in src/
    * Any specification item without a man page (suggest: `mfb man
      <package> <name>`)
12. Perform another review cycle (return to step 9)

COMPLETION:
Task complete when Phase 2 produces 3 consecutive reviews with zero
new FIXME items.

COMPLETION CHECKPOINT:
- Phase 0: All existing FIXMEs validated ✓
- Phase 1: ALL items in review_list.txt marked [DONE] ✓
- Phase 2: 3 consecutive reviews with zero new FIXMEs ✓

IMPORTANT: Do not stop between phases or cycles. Continue working until
final completion.
