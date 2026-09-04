# FE-01: a left-associative operator chain overflows the compiler's stack.
# MAX_EXPR_DEPTH does not bound left-recursion / postfix-member chains.
n = 20000
print('IMPORT io')
print('FUNC main() AS Integer')
print('  LET x AS Integer = 1' + '+1' * n)
print('  RETURN 0')
print('END FUNC')
