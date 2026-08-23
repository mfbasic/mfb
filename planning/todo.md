Cleaned up codegen

- = Not reviewed
+ = Started
@ = Reviewed

[-] app
[@] astrings
[-] audio
[@] bits
[@] collections
[@] crypto
[@] csv
[@] datetime
[@] encoding
[@] errorcode
[-] fs
[-] http
[@] io
[@] json
[-] math
[-] money
[-] net
[-] os
[-] process
[@] regex
[@] strings
[-] term
[-] thread
[-] tls
[-] vector


---

Next, and finally.

I want you to update tests/acceptance/regex.mfb

* I want to test every function
* I want you to TRAP and verify every possible error
* I want all known edge cases tested.

I dont want you to make them "just pass" I want you to put expected values and verify. If you find bugs, fix them. This is the full regex acceptance test suite. I am expecting multiple tests for each function. Consider this a full acceptance and security test suite for the regex import.

only after all the tests are written and working on macos, build for linux and windows and run the acceptance tests on their platforms.
