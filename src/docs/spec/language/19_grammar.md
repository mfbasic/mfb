# 19. Grammar (EBNF, abridged)

```ebnf
(* The parser runs one flat top-level loop; imports, declarations, LINK blocks,
   RESOURCE/FUNC-alias declarations, and DOC blocks may appear in any order. *)
program        = { import | declaration } ;

import         = "IMPORT" ( ident | qualifiedName ) [ "AS" ident ] ;
qualifiedName  = ident "::" ident ;
resourceDecl   = declVis "RESOURCE" ident "CLOSE" "BY" qualifiedName
                   [ "THREAD_SENDABLE" ] ;
funcAlias      = declVis "FUNC" ident "AS" qualifiedName ;
linkDecl       = "LINK" string "AS" ident { nativeFuncDecl | cstructDecl }
                   "END" "LINK" ;
cstructDecl    = "CSTRUCT" ident "AS" ident
                   { ident nativeType }
                   "END" "CSTRUCT" ;
(* CSTRUCT declares a C struct layout and the MFBASIC record it maps to. It is
   legal only inside a LINK block; naming it anywhere else is
   NATIVE_CSTRUCT_ESCAPE. `CSTRUCT` and the closing `END CSTRUCT` name are
   contextual identifiers, not keywords. *)
(* The native FUNC name may be a keyword (e.g. `step`, colliding with `STEP`);
   the parser accepts a keyword token in this position. A `RES` native return may
   carry a `STATE T` clause (plan-53): the native producer hands back a resource
   RECORD carrying a `T` payload, populated by BIND STATE (below) or
   default-initialized. STATE is honored only after `RES`, as on ordinary funcs. *)
nativeFuncDecl = "FUNC" name "(" [ params ] ")"
                   [ "AS" [ "RES" ] type [ "STATE" type ] ]
                   nativeFuncBody "END" "FUNC" ;
name           = ident | keyword ;
(* The body clauses may appear in any order; SYMBOL and ABI are required. There
   is no RETURN_OUT clause in the parser (multi-OUT is a deferred design, §17). *)
nativeFuncBody = { "SYMBOL" string
                 | "ABI" "(" [ abiSlotList ] ")" "AS" abiReturn
                 | constPin
                 | nativeReturnRule
                 | "RETURN" expr
                 | bindIn
                 | bindState
                 | nativeFree } ;
constPin       = "CONST" ident "=" ( "SIZEOF" ident | expr ) ;
(* The SIZEOF form pins a CSTRUCT's computed size; its operand is a CSTRUCT
   *type name*, not a value, so `expr` does not cover it. `SIZEOF` is a
   contextual identifier. *)
nativeReturnRule = "SUCCESS_ON" expr | "ERROR_ON" expr ;
(* BIND IN writes named struct fields into an IN/INOUT slot before the call;
   BIND STATE (plan-53) marshals a filled OUT struct slot into the returned
   resource's STATE payload after the call. `<res-slot>` is the native return
   naming the produced resource; `<struct-slot>` is an OUT ABI slot whose
   `CSTRUCT ... AS S` gives the resource's STATE type S. *)
bindIn         = "BIND" "IN" ident { ident "=" expr } "END" "BIND" ;
bindState      = "BIND" "STATE" ident "=" ident ;
(* In a FREE block both clauses may appear in any order; the deallocator's
   ABI return after `AS` is a bare nativeType (no slot name). *)
nativeFree     = "FREE" ident
                   { "SYMBOL" string
                   | "ABI" "(" abiSlot ")" "AS" nativeType }
                   "END" "FREE" ;
abiSlotList    = abiSlot { "," abiSlot } ;
abiSlot        = ident [ "IN" | "OUT" | "INOUT" ] nativeType ;
(* The native-return slot (after `AS`) accepts no direction modifier — just a
   slot name and a C type. An `OUT` result is instead an `abiSlot` carrying the
   `OUT` direction inside the slot list, surfaced by naming it in `RETURN`.
   bug-300 E4: these three productions used to admit a literal `return` as a slot
   name. plan-50-H deleted that special case — `return` lexes as `Keyword::Return`
   and `parse_abi_slot_name` accepts only an identifier, so it is rejected in all
   three positions — and `abiSlot` omitted the `IN`/`INOUT` directions plan-50-E
   added alongside `OUT`. *)
abiReturn      = ident nativeType ;
(* The ABI slot type is lexed as a free identifier; only the names below are
   honored by the marshaling backend (§17). *)
nativeType     = "CInt8" | "CInt16" | "CInt32" | "CInt64"
                | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
                | "CBool" | "CByte" | "CFloat" | "CDouble"
                | "CString" | "CPtr" | "CVoid" ;

declaration    = topLetDecl | topMutDecl
               | funcDecl | subDecl | typeDecl | unionDecl | enumDecl
               | resourceDecl | funcAlias | linkDecl
               | docBlock | testingBlock ;

(* A DOC block's body is captured verbatim by the LEXER as a single token rather
   than tokenized as code (§2.4), so `docBody` is not decomposed here; the block's
   internal structure and its rendering are specified in §21. The header line may
   carry only whitespace-separated attribute words (`DOC INTERNAL`); anything else
   makes the lexer roll back and treat `DOC` as an ordinary identifier. *)
docBlock       = "DOC" { ident } docBody "END" "DOC" ;

(* TESTING is a reserved keyword; TGROUP and TCASE are contextual identifiers, so
   both remain usable as ordinary names everywhere else. TGROUPs nest, bounded by
   the same statement-nesting cap control flow uses. *)
testingBlock   = "TESTING" { testGroup } "END" "TESTING" ;
testGroup      = "TGROUP" string { testGroup | testCase } "END" "TGROUP" ;
testCase       = "TCASE" string block "END" "TCASE" ;

declVis        = [ "EXPORT" | "PUBLIC" | "PRIVATE" ] ;
funcIso        = [ "ISOLATED" ] ;

topLetDecl     = declVis "LET" ident [ "AS" type ] "=" expr ;
topMutDecl     = declVis "MUT" ident [ "AS" type ] [ "=" expr ] ;

(* Both the parameter list and the return type are optional: `FUNC f AS Integer`
   (no parens) and `FUNC f()` (no `AS`) each parse. A `FUNC` with no returnType
   yields Nothing, exactly as a SUB does. *)
funcDecl       = declVis funcIso "FUNC" ident [ templateParams ] [ "(" [ params ] ")" ]
                   [ returnType ] block [ trap ] "END" "FUNC" ;
subDecl        = declVis "SUB" ident [ templateParams ] [ "(" [ params ] ")" ]
                   block [ trap ] "END" "SUB" ;
trap           = "TRAP" [ "(" ident ")" ] block "END" "TRAP" ;
(* The `(ident)` error binding is OPTIONAL. A bare `TRAP` runs the handler
   without naming the caught error; `PROPAGATE` still re-raises it. The same
   optional binding applies to the inline postfix trap (§8.4, prose): the
   `expr TRAP … END TRAP` attached to a LET/MUT binding, assignment, or
   bare-expression statement may likewise omit `(ident)`. *)

templateParams = "OF" ident { "," ident } ;
params         = param { "," param } ;
(* `RES` marks a resource parameter; `STATE T` attaches a typed STATE payload
   to a resource binding (§15) and is parsed ONLY when `RES` is present. The
   `AS type` clause is syntactically optional in the parser. *)
param          = [ "RES" ] ident [ "AS" type ] [ "STATE" type ] [ "=" expr ] ;
(* STATE in a returnType is likewise honored only when `RES` is present. *)
returnType     = "AS" [ "RES" ] type [ "STATE" type ] ;
type           = templateType | funcType | "(" type ")" | ident | qualifiedIdent ;
typeList       = type { "," type } ;
(* `RES` markers denote resource-transfer collections / thread planes (§15.6).
   `Result OF type` is COMPILER-INTERNAL: it is the private fallible-outcome form
   (§4.4, §8.8) and is never written in user source — the parser only emits it
   during desugaring. *)
templateType
               = ( "Map" | "MapEntry" ) "OF" type "TO" [ "RES" ] type
               | ( "List" ) "OF" [ "RES" ] type
               | "Result" "OF" type                       (* internal only; not user-writable *)
               | ( "Thread" | "ThreadWorker" ) "OF" threadBody
               | (ident | qualifiedIdent) "OF" type { "," type } ;
threadBody     = [ type ] [ "RES" type ] "TO" type ;  (* message defaults to Nothing *)
funcType       = [ "ISOLATED" ] "FUNC" "(" [ typeList ] ")" "AS" type ;

typeDecl       = declVis "TYPE" ident [ templateParams ] { field } "END" "TYPE" ;
field          = declVis ident "AS" type ;
unionDecl      = declVis "UNION" ident [ templateParams ] [ unionIncludes ] { unionMember } "END" "UNION" ;
unionIncludes  = "INCLUDES" unionName { "," unionName } ;
unionName      = ident | qualifiedIdent ;
unionMember    = ident | qualifiedIdent ;
enumDecl       = declVis "ENUM" ident identlist "END" "ENUM" ;
identlist      = ident { "," ident } ;

block          = { statement } ;
statement      = letStmt | mutStmt | assignStmt
               | ifStmt | forStmt | foreachStmt | whileStmt
               | doStmt | matchStmt
               | failStmt | propagateStmt | recoverStmt | returnStmt
               | exitStmt | continueStmt
               | exprStmt | "REM" ... ;

letStmt        = "LET" ident [ "AS" type ] "=" expr ;
mutStmt        = "MUT" ident [ "AS" type ] [ "=" expr ] ;
(* `ident.state` / `ident.state.field` are the only member-target assignments —
   they replace a RES binding's STATE payload (§15). *)
assignStmt     = ident "=" expr
               | ident "." "state" "=" expr
               | ident "." "state" "." ident "=" expr ;
recoverStmt    = "RECOVER" [ expr ] ;

(* Semantic rule: MUT without an initializer requires an explicit type
   with a defined default value. *)

ifStmt         = inlineIfStmt | blockIfStmt ;
inlineIfStmt   = "IF" expr "THEN" simpleStmt [ "ELSE" simpleStmt ] ;
blockIfStmt    = "IF" expr "THEN" block
                   { "ELSEIF" expr "THEN" block }
                   [ "ELSE" block ]
                   "END" "IF" ;
simpleStmt     = letStmt | mutStmt | assignStmt | failStmt | propagateStmt
               | recoverStmt | returnStmt | exitStmt | continueStmt | exprStmt ;
forStmt        = "FOR" ident "=" expr "TO" expr [ "STEP" expr ]
                   block "NEXT" ;
foreachStmt    = "FOR" "EACH" ident "IN" expr block "NEXT" ;
whileStmt      = "WHILE" expr block "WEND" ;
doStmt         = "DO" block "LOOP" "UNTIL" expr
               | "DO" "WHILE" expr block "LOOP" ;

failStmt       = "FAIL" expr ;
propagateStmt  = "PROPAGATE" ;
returnStmt     = "RETURN" [ expr ] ;
exitStmt       = "EXIT" loopKind | "EXIT" "SUB" | "EXIT" "FUNC"
               | "EXIT" "PROGRAM" expr ;
continueStmt   = "CONTINUE" loopKind ;
loopKind       = "FOR" | "DO" | "WHILE" ;
exprStmt       = expr ;

matchStmt      = "MATCH" expr { caseClause } "END" "MATCH" ;
(* The CASE pattern is ended by the line/statement terminator (not a `:`); the
   body block runs until the next CASE or END MATCH. *)
caseClause     = "CASE" ( "ELSE" | patternList ) [ "WHEN" expr ] block ;
patternList    = pattern { "," pattern } ;
pattern        = unionPattern | expr ;       (* expr covers enum members and literals *)
unionPattern   = (ident | qualifiedIdent) "(" ident ")" ;

(* Pipe: the right-hand side of `|>` is a full orExpr that must contain at least
   one `_` placeholder; the left operand is substituted for every `_` (it is not
   a restricted call form). *)
expr           = orExpr { "|>" orExpr } ;
orExpr         = andExpr { ("OR" | "XOR") andExpr } ;   (* OR and XOR share a level *)
andExpr        = notExpr { "AND" notExpr } ;
notExpr        = "NOT" notExpr | cmpExpr ;              (* NOT chains right *)
cmpExpr        = concatExpr { cmpOp concatExpr } ;
cmpOp          = "=" | "<>" | "<" | ">" | "<=" | ">=" ;
concatExpr     = addExpr { "&" addExpr } ;             (* `&` binds looser than +/- *)
addExpr        = mulExpr { ("+"|"-") mulExpr } ;
mulExpr        = powExpr { ("*"|"/"|"DIV"|"MOD") powExpr } ;
powExpr        = unary [ "^" powExpr ] ;               (* right-associative *)
unary          = "-" unary | withExpr | memberAccess ; (* unary minus only; no unary + *)
withExpr       = "WITH" memberAccess "{" fieldAssigns "}" ;
memberAccess   = callOrCtor { "." ident } ;
callOrCtor     = primary { "(" [ callArgList ] ")" | "[" [ callArgList ] "]" } ;
primary        = literal | ident | qualifiedIdent | lambda
               | enumMember | listLit | mapLit
               | "(" expr ")" ;
literal        = integer | decimal | string | scalar | "TRUE" | "FALSE" | "NOTHING" ;
scalar         = "`" ( scalarChar | escape ) "`" ; (* one Unicode scalar; §2.3 *)

qualifiedIdent = ident "::" ident ;         (* package::identifier only *)
enumMember     = ident "." ident ;         (* EnumType.Member *)
                                                (* Name resolution disambiguates
                                                   ident.ident: type name on
                                                   left => enum member; value on
                                                   left => field access. *)
(* A call `f(...)` and a constructor `T[...]` are the `callOrCtor` postfixes
   above. The parser restricts the head to a bare ident or qualifiedIdent — only
   `f(...)` and `T[...]` are accepted, not `(expr)(...)`. A constructor `[...]`
   and a call `(...)` share callArgList, so positional and `name :=` named
   arguments are accepted in both. *)
callArgList    = callArg { "," callArg } ;
callArg        = [ ident ":=" ] expr ;            (* `_` is an ordinary primary used as the pipe placeholder *)
lambda         = "LAMBDA" "(" [ params ] ")" "->" ( ident "=" expr | expr ) ;
                 (* the `ident = expr` body mutates a captured MUT binding and yields Nothing *)
fieldAssigns   = fieldAssign { "," fieldAssign } ;
fieldAssign    = ident ":=" expr ;
listLit        = "[" [ exprList ] "]" ;
exprList       = expr { "," expr } ;
mapLit         = "Map" "OF" type "TO" type "{" [ mapEntries ] "}" ;
mapEntries     = mapEntry { "," mapEntry } ;
mapEntry       = expr ":=" expr ;
```

## See Also

* ./mfb spec language lexical-structure — the tokens the grammar's terminals refer to
* ./mfb spec language operators — operator precedence not encoded in this abridged grammar
* ./mfb spec architecture frontend — the parser that implements this grammar
