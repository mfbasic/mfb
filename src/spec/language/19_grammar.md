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
linkDecl       = "LINK" string "AS" ident { nativeFuncDecl } "END" "LINK" ;
nativeFuncDecl = "FUNC" ident "(" [ params ] ")" [ "AS" [ "RES" ] type ]
                   nativeFuncBody "END" "FUNC" ;
(* The body clauses may appear in any order; SYMBOL and ABI are required. There
   is no RETURN_OUT clause in the parser (multi-OUT is a deferred design, §17). *)
nativeFuncBody = { "SYMBOL" string
                 | "ABI" "(" [ abiSlotList ] ")" "AS" abiSlot
                 | constPin
                 | nativeReturnRule
                 | "RESULT" expr
                 | nativeFree } ;
constPin       = "CONST" ident "=" expr ;
nativeReturnRule = "SUCCESS_ON" expr | "ERROR_ON" expr ;
nativeFree     = "FREE" ( ident | "return" )
                   "SYMBOL" string
                   "ABI" "(" abiSlot ")" "AS" nativeType
                   "END" "FREE" ;
abiSlotList    = abiSlot { "," abiSlot } ;
abiSlot        = ( ident | "return" ) [ "OUT" ] nativeType ;
(* The ABI slot type is lexed as a free identifier; only the names below are
   honored by the marshaling backend (§17). *)
nativeType     = "CInt8" | "CInt16" | "CInt32" | "CInt64"
                | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
                | "CBool" | "CByte" | "CFloat" | "CDouble"
                | "CString" | "CPtr" | "CVoid" ;

declaration    = topLetDecl | topMutDecl
               | funcDecl | subDecl | typeDecl | unionDecl | enumDecl
               | resourceDecl | funcAlias | linkDecl ;

declVis        = [ "EXPORT" | "PACKAGE" | "PRIVATE" ] ;
funcIso        = [ "ISOLATED" ] ;

topLetDecl     = declVis "LET" ident [ "AS" type ] "=" expr ;
topMutDecl     = declVis "MUT" ident [ "AS" type ] [ "=" expr ] ;

funcDecl       = declVis funcIso "FUNC" ident [ templateParams ] "(" [ params ] ")" returnType
                   block [ trap ] "END" "FUNC" ;
subDecl        = declVis "SUB" ident [ templateParams ] "(" [ params ] ")"
                   block [ trap ] "END" "SUB" ;
trap           = "TRAP" ident block "END" "TRAP" ;

templateParams = "OF" ident { "," ident } ;
params         = param { "," param } ;
(* `RES` marks a resource parameter; `STATE T` attaches a typed STATE payload
   to a resource binding (§15). *)
param          = [ "RES" ] ident "AS" type [ "STATE" type ] [ "=" expr ] ;
returnType     = "AS" [ "RES" ] type [ "STATE" type ] ;
type           = templateType | funcType | "(" type ")" | ident | qualifiedIdent ;
typeList       = type { "," type } ;
(* `RES` markers denote resource-transfer collections / thread planes (§15.6). *)
templateType
               = ( "Map" | "MapEntry" ) "OF" type "TO" [ "RES" ] type
               | ( "List" ) "OF" [ "RES" ] type
               | "Result" "OF" type
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
literal        = integer | decimal | string | "TRUE" | "FALSE" | "NOTHING" ;

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
