okay, I want to start adding the generic functions to the compiler now. read "## 12. Built-in JSON Package" in the spec.

I'm looking for a minimum 2 tests for each function a func-json-*-valid and func-json-*-invalid. 
Valid test should test all usage patterns and overloads for the function, edge-cases included.

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
