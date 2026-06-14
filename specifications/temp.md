okay, I want to start adding the generic functions to the compiler now. read "## 10. Built-in Math Package" in the spec.

I'm looking for a minimum 2 tests for each function a func-math-*-valid and func-math-*-invalid. 
Valid test should test all usage patterns and overloads for the function, edge-cases included.

for ArchLinux aarch64 glibc: ssh -p 2222 test@127.0.0.1
for Kali aarch64 glibc: ssh -p 2223 test@127.0.0.1
for Apline aarch64 musl: ssh -p 2224 test@127.0.0.1

If you need more thant 2 tests, add them.

1) Review the spec "## 10. Built-in Math Package"
2) implement a single function including all it's overloads
3) Add the test files (valid and invalid)
4) Add or Verify src/man/math/<func>.txt is correct
5) Run the `mfb man` to view the help file
6) Run all tests
7) goto #1 until all "## 10. Built-in Math Package" are implemented

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
