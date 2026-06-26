# 19. Grammar (EBNF, abridged)

```ebnf
program        = { import | linkDecl } { declaration } ;

import         = "IMPORT" ident [ "AS" ident ] ;
qualifiedName  = ident "::" ident ;
resourceDecl   = declVis "RESOURCE" ident "CLOSE" "BY" qualifiedName
                   [ "THREAD_SENDABLE" ] ;
funcAlias      = declVis "FUNC" ident "AS" qualifiedName ;
linkDecl       = "LINK" string "AS" ident { nativeFuncDecl } "END" "LINK" ;
nativeFuncDecl = "FUNC" ident "(" [ params ] ")" [ "AS" [ "RES" ] type ]
                   nativeFuncBody "END" "FUNC" ;
nativeFuncBody = "SYMBOL" string
                   "ABI" "(" [ abiSlotList ] ")" "AS" abiSlot
                   { constPin }
                   [ nativeReturnRule ]
                   [ "RESULT" expr ]
                   [ returnOut ]
                   { nativeFree } ;
constPin       = "CONST" ident "=" expr ;
nativeReturnRule = "SUCCESS_ON" expr | "ERROR_ON" expr ;
returnOut       = "RETURN_OUT" ident "[" ident { "," ident } "]" ;
nativeFree     = "FREE" ( ident | "return" )
                   "SYMBOL" string
                   "ABI" "(" abiSlot ")" "AS" nativeType
                   "END" "FREE" ;
abiSlotList    = abiSlot { "," abiSlot } ;
abiSlot        = ( ident | "return" ) [ "OUT" ] nativeType ;
nativeType     = "CInt8" | "CInt16" | "CInt32" | "CInt64"
                | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
                | "CBool" | "CFloat32" | "CFloat64"
                | "CIntPtr" | "CUIntPtr" | "CSize"
                | "CString" | "CPtr" | "CVoid" ;

declaration    = topLetDecl | topMutDecl
               | funcDecl | subDecl | typeDecl | unionDecl | enumDecl
               | resourceDecl | funcAlias | linkDecl ;

declVis        = [ "EXPORT" | "PACKAGE" | "PRIVATE" ] ;
funcIso        = [ "ISOLATED" ] ;

topLetDecl     = declVis "LET" ident [ "AS" type ] "=" expr ;
topMutDecl     = declVis "MUT" ident [ "AS" type ] [ "=" expr ] ;

funcDecl       = declVis funcIso "FUNC" ident [ templateParams ] "(" [ params ] ")" "AS" type
                   block [ trap ] "END" "FUNC" ;
subDecl        = declVis "SUB" ident [ templateParams ] "(" [ params ] ")"
                   block [ trap ] "END" "SUB" ;
trap           = "TRAP" ident block "END" "TRAP" ;

templateParams = "OF" ident { "," ident } ;
params         = param { "," param } ;
param          = ident "AS" type [ "=" expr ] ;
type           = templateType | funcType | ident | qualifiedIdent ;
typeList       = type { "," type } ;
templateType
               = "Map" "OF" type "TO" type
               | "Thread" "OF" type "TO" type
               | (ident | qualifiedIdent) "OF" type { "," type } ;
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
               | failStmt | propagateStmt | returnStmt
               | exprStmt | "REM" ... ;

letStmt        = "LET" ident [ "AS" type ] "=" expr ;
mutStmt        = "MUT" ident [ "AS" type ] [ "=" expr ] ;
assignStmt     = ident "=" expr ;

(* Semantic rule: MUT without an initializer requires an explicit type
   with a defined default value. *)

ifStmt         = inlineIfStmt | blockIfStmt ;
inlineIfStmt   = "IF" expr "THEN" simpleStmt [ "ELSE" simpleStmt ] ;
blockIfStmt    = "IF" expr "THEN" block
                   { "ELSEIF" expr "THEN" block }
                   [ "ELSE" block ]
                   "END" "IF" ;
simpleStmt     = letStmt | mutStmt | assignStmt | failStmt | propagateStmt
               | returnStmt | exitStmt | continueStmt | exprStmt ;
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
caseClause     = "CASE" patternList [ "WHEN" expr ] ":" block
               | "CASE" "ELSE" ":" block ;
patternList    = pattern { "," pattern } ;
pattern        = enumMember | unionPattern | literal ;
unionPattern   = (ident | qualifiedIdent) "(" ident ")" ;

expr           = orExpr { "|>" pipeTail } ;
pipeTail       = (ident | qualifiedIdent) "(" [ pipeArgList ] ")" ;
pipeArgList    = pipeArg { "," pipeArg } ;
pipeArg        = [ ident ":=" ] ( expr | "_" ) ;
orExpr         = andExpr { ("OR" | "XOR") andExpr } ;
andExpr        = notExpr { "AND" notExpr } ;
notExpr        = [ "NOT" ] cmpExpr ;
cmpExpr        = addExpr { cmpOp addExpr } ;
cmpOp          = "=" | "<>" | "<" | ">" | "<=" | ">=" ;
addExpr        = mulExpr { ("+"|"-"|"&") mulExpr } ;
mulExpr        = powExpr { ("*"|"/"|"DIV"|"MOD") powExpr } ;
powExpr        = unary [ "^" powExpr ] ;       (* right-associative *)
unary          = [ "-" ] fieldAccess ;
fieldAccess    = primary { "." ident } ;
primary        = literal | ident | qualifiedIdent | call | lambda
               | enumMember | constructor | withExpr | listLit | mapLit
               | "(" expr ")" ;
literal        = integer | decimal | string | "TRUE" | "FALSE" | "NOTHING" ;

qualifiedIdent = ident "::" ident ;         (* package::identifier only *)
enumMember     = ident "." ident ;         (* EnumType.Member *)
                                                (* Name resolution disambiguates
                                                   ident.ident: type name on
                                                   left => enum member; value on
                                                   left => field access. *)
call           = (ident | qualifiedIdent) "(" [ callArgList ] ")" ;
callArgList    = callArg { "," callArg } ;
callArg        = [ ident ":=" ] expr ;
lambda         = "LAMBDA" "(" [ params ] ")" "->" expr ;
withExpr       = "WITH" expr "{" fieldAssigns "}" ;
fieldAssigns   = fieldAssign { "," fieldAssign } ;
fieldAssign    = ident ":=" expr ;
constructor    = (ident | qualifiedIdent) "[" [ callArgList ] "]" ;
listLit        = "[" [ exprList ] "]" ;
exprList       = expr { "," expr } ;
mapLit         = "Map" "OF" type "TO" type "{" [ mapEntries ] "}" ;
mapEntries     = mapEntry { "," mapEntry } ;
mapEntry       = expr ":=" expr ;
```
