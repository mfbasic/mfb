pub(crate) struct PackageDoc {
    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) usage: &'static str,
    pub(crate) functions: &'static [FunctionDoc],
}

pub(crate) struct FunctionDoc {
    pub(crate) name: &'static str,
    pub(crate) signature: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) example: &'static str,
}

const GENERAL_FUNCTIONS: &[FunctionDoc] = &[
    FunctionDoc {
        name: "len",
        signature: "len(value AS String | List OF T | Map OF K TO V) AS Integer",
        summary: "Returns the number of characters, list items, or map entries.",
        example: "LET count AS Integer = len(\"hello\")",
    },
    FunctionDoc {
        name: "find",
        signature: "find(value, needle[, start AS Integer]) AS Integer",
        summary: "Returns the first zero-based position of a substring or list item, optionally starting at an index.",
        example: "LET index AS Integer = find(\"hello\", \"l\")",
    },
    FunctionDoc {
        name: "mid",
        signature: "mid(value AS String | List OF T, start AS Integer, count AS Integer)",
        summary: "Returns a slice from a string or list.",
        example: "LET part AS String = mid(\"hello\", 1, 3)",
    },
    FunctionDoc {
        name: "replace",
        signature: "replace(value, old, new)",
        summary: "Returns a string or list with matching values replaced.",
        example: "LET text AS String = replace(\"hello\", \"l\", \"x\")",
    },
    FunctionDoc {
        name: "typeName",
        signature: "typeName(value AS T) AS String",
        summary: "Returns the static MFBASIC type name of a value.",
        example: "LET name AS String = typeName(42)",
    },
    FunctionDoc {
        name: "toString",
        signature: "toString(value AS Integer | Float | Fixed | Boolean | String | Byte | List OF Byte) AS String",
        summary: "Converts a primitive value or byte list to a string.",
        example: "LET text AS String = toString(42)",
    },
    FunctionDoc {
        name: "toInt",
        signature: "toInt(value AS String | Float | Fixed) AS Integer",
        summary: "Converts text or a numeric value to an integer.",
        example: "LET value AS Integer = toInt(\"42\")",
    },
    FunctionDoc {
        name: "toFloat",
        signature: "toFloat(value AS String | Integer | Fixed) AS Float",
        summary: "Converts text or a numeric value to a float.",
        example: "LET value AS Float = toFloat(\"1.5\")",
    },
    FunctionDoc {
        name: "toFixed",
        signature: "toFixed(value AS String | Integer | Float) AS Fixed",
        summary: "Converts text or a numeric value to a fixed-point value.",
        example: "LET value AS Fixed = toFixed(\"2.25\")",
    },
    FunctionDoc {
        name: "toByte",
        signature: "toByte(value AS Integer) AS Byte",
        summary: "Converts an integer to a byte.",
        example: "LET value AS Byte = toByte(65)",
    },
    FunctionDoc {
        name: "isNumeric",
        signature: "isNumeric(value AS String) AS Boolean",
        summary: "Returns TRUE when text can be parsed as a number.",
        example: "LET ok AS Boolean = isNumeric(\"42\")",
    },
    FunctionDoc {
        name: "isEven",
        signature: "isEven(value AS Integer) AS Boolean",
        summary: "Returns TRUE when an integer is even.",
        example: "LET ok AS Boolean = isEven(4)",
    },
    FunctionDoc {
        name: "isOdd",
        signature: "isOdd(value AS Integer) AS Boolean",
        summary: "Returns TRUE when an integer is odd.",
        example: "LET ok AS Boolean = isOdd(3)",
    },
    FunctionDoc {
        name: "isPositive",
        signature: "isPositive(value AS Integer | Float | Fixed) AS Boolean",
        summary: "Returns TRUE when a number is greater than zero.",
        example: "LET ok AS Boolean = isPositive(1)",
    },
    FunctionDoc {
        name: "isNegative",
        signature: "isNegative(value AS Integer | Float | Fixed) AS Boolean",
        summary: "Returns TRUE when a number is less than zero.",
        example: "LET ok AS Boolean = isNegative(-1)",
    },
    FunctionDoc {
        name: "isZero",
        signature: "isZero(value AS Integer | Float | Fixed) AS Boolean",
        summary: "Returns TRUE when a number is zero.",
        example: "LET ok AS Boolean = isZero(0)",
    },
    FunctionDoc {
        name: "isEmpty",
        signature: "isEmpty(value AS String | List OF T | Map OF K TO V) AS Boolean",
        summary: "Returns TRUE when a string, list, or map has no contents.",
        example: "LET ok AS Boolean = isEmpty(\"\")",
    },
    FunctionDoc {
        name: "isNotEmpty",
        signature: "isNotEmpty(value AS String | List OF T | Map OF K TO V) AS Boolean",
        summary: "Returns TRUE when a string, list, or map has contents.",
        example: "LET ok AS Boolean = isNotEmpty(\"hello\")",
    },
    FunctionDoc {
        name: "get",
        signature: "get(collection AS List OF T | Map OF K TO V, key) AS T | V",
        summary: "Returns a value from a list index or map key.",
        example: "LET value AS Integer = get([1, 2, 3], 0)",
    },
    FunctionDoc {
        name: "getOr",
        signature: "getOr(collection AS List OF T | Map OF K TO V, key, fallback) AS T | V",
        summary: "Returns a value from a collection or the fallback when it is missing.",
        example: "LET value AS Integer = getOr([1, 2, 3], 9, 0)",
    },
    FunctionDoc {
        name: "set",
        signature: "set(collection AS List OF T | Map OF K TO V, key, value)",
        summary: "Returns a collection with one list index or map key set to a new value.",
        example: "LET numbers AS List OF Integer = set([1, 2, 3], 1, 9)",
    },
    FunctionDoc {
        name: "append",
        signature: "append(values AS List OF T, value AS T | List OF T) AS List OF T",
        summary: "Returns a list with a value or another list appended.",
        example: "LET numbers AS List OF Integer = append([1, 2], 3)",
    },
    FunctionDoc {
        name: "prepend",
        signature: "prepend(values AS List OF T, value AS T) AS List OF T",
        summary: "Returns a list with a value inserted at the front.",
        example: "LET numbers AS List OF Integer = prepend([2, 3], 1)",
    },
    FunctionDoc {
        name: "insert",
        signature: "insert(values AS List OF T, index AS Integer, value AS T) AS List OF T",
        summary: "Returns a list with a value inserted at an index.",
        example: "LET numbers AS List OF Integer = insert([1, 3], 1, 2)",
    },
    FunctionDoc {
        name: "removeAt",
        signature: "removeAt(values AS List OF T, index AS Integer) AS List OF T",
        summary: "Returns a list with one index removed.",
        example: "LET numbers AS List OF Integer = removeAt([1, 2, 3], 1)",
    },
    FunctionDoc {
        name: "removeKey",
        signature: "removeKey(values AS Map OF K TO V, key AS K) AS Map OF K TO V",
        summary: "Returns a map with one key removed.",
        example: "LET scores AS Map OF String TO Integer = removeKey(Map OF String TO Integer { \"a\" := 1 }, \"a\")",
    },
    FunctionDoc {
        name: "keys",
        signature: "keys(values AS Map OF K TO V) AS List OF K",
        summary: "Returns the keys from a map.",
        example: "LET names AS List OF String = keys(Map OF String TO Integer { \"a\" := 1 })",
    },
    FunctionDoc {
        name: "values",
        signature: "values(values AS Map OF K TO V) AS List OF V",
        summary: "Returns the values from a map.",
        example: "LET scores AS List OF Integer = values(Map OF String TO Integer { \"a\" := 1 })",
    },
    FunctionDoc {
        name: "hasKey",
        signature: "hasKey(values AS Map OF K TO V, key AS K) AS Boolean",
        summary: "Returns TRUE when a map contains a key.",
        example: "LET ok AS Boolean = hasKey(Map OF String TO Integer { \"a\" := 1 }, \"a\")",
    },
    FunctionDoc {
        name: "contains",
        signature: "contains(values AS List OF T, value AS T) AS Boolean",
        summary: "Returns TRUE when a list contains a value.",
        example: "LET ok AS Boolean = contains([1, 2, 3], 2)",
    },
    FunctionDoc {
        name: "forEach",
        signature: "forEach(values AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing",
        summary: "Runs a function for each item in a list.",
        example: "forEach([\"hello\"], io.print)",
    },
    FunctionDoc {
        name: "transform",
        signature: "transform(values AS List OF T, mapper AS FUNC(T) AS U) AS List OF U",
        summary: "Returns a list created by applying a function to each item.",
        example: "LET text AS List OF String = transform([1, 2], toString)",
    },
    FunctionDoc {
        name: "filter",
        signature: "filter(values AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T",
        summary: "Returns list items that pass a predicate.",
        example: "LET evens AS List OF Integer = filter([1, 2, 3], isEven)",
    },
    FunctionDoc {
        name: "reduce",
        signature: "reduce(values AS List OF T, initial AS U, reducer AS FUNC(U, T) AS U) AS U",
        summary: "Combines list items into one accumulated value.",
        example: "LET total AS Integer = reduce([1, 2, 3], 0, add)",
    },
    FunctionDoc {
        name: "sum",
        signature: "sum(values AS List OF Integer | List OF Float | List OF Fixed)",
        summary: "Returns the total of a numeric list.",
        example: "LET total AS Integer = sum([1, 2, 3])",
    },
];

const IO_FUNCTIONS: &[FunctionDoc] = &[
    FunctionDoc {
        name: "print",
        signature: "io.print(value AS String) AS Nothing",
        summary: "Writes a string to standard output followed by a newline.",
        example: "io.print(\"hello\")",
    },
    FunctionDoc {
        name: "write",
        signature: "io.write(value AS String) AS Nothing",
        summary: "Writes a string to standard output without adding a newline.",
        example: "io.write(\"hello\")",
    },
    FunctionDoc {
        name: "printError",
        signature: "io.printError(value AS String) AS Nothing",
        summary: "Writes a string to standard error followed by a newline.",
        example: "io.printError(\"failed\")",
    },
    FunctionDoc {
        name: "writeError",
        signature: "io.writeError(value AS String) AS Nothing",
        summary: "Writes a string to standard error without adding a newline.",
        example: "io.writeError(\"failed\")",
    },
    FunctionDoc {
        name: "flush",
        signature: "io.flush() AS Nothing",
        summary: "Flushes standard output.",
        example: "io.flush()",
    },
    FunctionDoc {
        name: "flushError",
        signature: "io.flushError() AS Nothing",
        summary: "Flushes standard error.",
        example: "io.flushError()",
    },
    FunctionDoc {
        name: "input",
        signature: "io.input([prompt AS String]) AS String",
        summary: "Reads a line from standard input, optionally after writing a prompt.",
        example: "LET name AS String = io.input(\"Name: \")",
    },
    FunctionDoc {
        name: "readLine",
        signature: "io.readLine() AS String",
        summary: "Reads a line from standard input.",
        example: "LET line AS String = io.readLine()",
    },
    FunctionDoc {
        name: "readChar",
        signature: "io.readChar() AS String",
        summary: "Reads one character from standard input.",
        example: "LET char AS String = io.readChar()",
    },
    FunctionDoc {
        name: "readByte",
        signature: "io.readByte() AS Byte",
        summary: "Reads one byte from standard input.",
        example: "LET byte AS Byte = io.readByte()",
    },
    FunctionDoc {
        name: "isInputTerminal",
        signature: "io.isInputTerminal() AS Boolean",
        summary: "Returns TRUE when standard input is connected to a terminal.",
        example: "LET interactive AS Boolean = io.isInputTerminal()",
    },
    FunctionDoc {
        name: "isOutputTerminal",
        signature: "io.isOutputTerminal() AS Boolean",
        summary: "Returns TRUE when standard output is connected to a terminal.",
        example: "LET interactive AS Boolean = io.isOutputTerminal()",
    },
    FunctionDoc {
        name: "isErrorTerminal",
        signature: "io.isErrorTerminal() AS Boolean",
        summary: "Returns TRUE when standard error is connected to a terminal.",
        example: "LET interactive AS Boolean = io.isErrorTerminal()",
    },
    FunctionDoc {
        name: "terminalSize",
        signature: "io.terminalSize() AS TerminalSize",
        summary: "Returns the terminal column and row count.",
        example: "LET size AS TerminalSize = io.terminalSize()",
    },
];

const PACKAGES: &[PackageDoc] = &[
    PackageDoc {
        name: "general",
        summary: "Core functions for strings, numbers, collections, conversion, and predicates. These functions are available without an IMPORT.",
        usage: "Call general functions directly, for example len(\"hello\") or isEven(4).",
        functions: GENERAL_FUNCTIONS,
    },
    PackageDoc {
        name: "io",
        summary: "Terminal and standard stream input/output functions.",
        usage: "Add IMPORT io, then call functions with the io. prefix, for example io.print(\"hello\").",
        functions: IO_FUNCTIONS,
    },
];

pub(crate) fn packages() -> &'static [PackageDoc] {
    PACKAGES
}

pub(crate) fn package(name: &str) -> Option<&'static PackageDoc> {
    PACKAGES.iter().find(|package| package.name == name)
}

pub(crate) fn function(package: &PackageDoc, name: &str) -> Option<&'static FunctionDoc> {
    let local_name = name
        .strip_prefix(package.name)
        .and_then(|remaining| remaining.strip_prefix('.'))
        .unwrap_or(name);
    package
        .functions
        .iter()
        .find(|function| function.name == local_name)
}
