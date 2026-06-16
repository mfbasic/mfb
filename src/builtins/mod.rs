pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod math;
pub(crate) mod strings;
pub(crate) mod thread;

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(name, "fs" | "io" | "json" | "math" | "strings" | "thread")
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    fs::is_builtin_type(name)
        || io::is_builtin_type(name)
        || json::is_builtin_type(name)
        || thread::is_builtin_type(name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    fs::resource_close_function(type_name)
}

pub(crate) fn is_resource_type(type_name: &str) -> bool {
    resource_close_function(type_name).is_some()
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    strings::call_return_type_name(name)
        .or_else(|| math::call_return_type_name(name))
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
        .or_else(|| json::call_return_type_name(name))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    general::is_general_call(name)
        || strings::is_strings_call(name)
        || math::is_math_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || json::is_json_call(name)
        || thread::is_thread_call(name)
        || call_return_type_name(name).is_some()
}

pub(crate) fn is_builtin_member(name: &str) -> bool {
    is_builtin_call(name) || math::is_math_constant(name)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "len" => Some(&["value"]),
        "find" => Some(&["value", "needle", "start"]),
        "mid" => Some(&["value", "start", "count"]),
        "replace" => Some(&["value", "needle", "replacement"]),
        "typeName" => Some(&["value"]),
        "toString" => Some(&["value", "decimals"]),
        "toInt" => Some(&["value"]),
        "toFloat" => Some(&["value"]),
        "toFixed" => Some(&["value"]),
        "toByte" => Some(&["value"]),
        "isNumeric" => Some(&["value"]),
        "isEven" => Some(&["value"]),
        "isOdd" => Some(&["value"]),
        "isPositive" => Some(&["value"]),
        "isNegative" => Some(&["value"]),
        "isZero" => Some(&["value"]),
        "isEmpty" => Some(&["value"]),
        "isNotEmpty" => Some(&["value"]),
        "get" => Some(&["collection", "index"]),
        "getOr" => Some(&["collection", "index", "fallback"]),
        "set" => Some(&["collection", "index", "value"]),
        "append" => Some(&["list", "value"]),
        "prepend" => Some(&["list", "value"]),
        "insert" => Some(&["list", "index", "value"]),
        "removeAt" => Some(&["list", "index"]),
        "removeKey" => Some(&["map", "key"]),
        "keys" => Some(&["map"]),
        "values" => Some(&["map"]),
        "hasKey" => Some(&["map", "key"]),
        "contains" => Some(&["collection", "value"]),
        "forEach" => Some(&["collection", "action"]),
        "transform" => Some(&["collection", "transform"]),
        "filter" => Some(&["collection", "predicate"]),
        "reduce" => Some(&["collection", "seed", "combine"]),
        "sum" => Some(&["collection"]),
        "strings.trim" => Some(&["value"]),
        "strings.trimStart" => Some(&["value"]),
        "strings.trimEnd" => Some(&["value"]),
        "strings.upper" => Some(&["value"]),
        "strings.lower" => Some(&["value"]),
        "strings.caseFold" => Some(&["value"]),
        "strings.normalizeNfc" => Some(&["value"]),
        "strings.graphemes" => Some(&["value"]),
        "strings.startsWith" => Some(&["value", "prefix"]),
        "strings.endsWith" => Some(&["value", "suffix"]),
        "strings.contains" => Some(&["value", "needle"]),
        "strings.split" => Some(&["value", "separator"]),
        "strings.join" => Some(&["values", "separator"]),
        "strings.byteLen" => Some(&["value"]),
        "fs.fileExists" => Some(&["path"]),
        "fs.directoryExists" => Some(&["path"]),
        "fs.exists" => Some(&["path"]),
        "fs.readBytes" => Some(&["path"]),
        "fs.readText" => Some(&["path"]),
        "fs.writeBytes" => Some(&["path", "value"]),
        "fs.writeText" => Some(&["path", "value"]),
        "fs.writeBytesAtomic" => Some(&["path", "value"]),
        "fs.writeTextAtomic" => Some(&["path", "value"]),
        "fs.appendBytes" => Some(&["path", "value"]),
        "fs.appendText" => Some(&["path", "value"]),
        "fs.open" => Some(&["path", "mode"]),
        "fs.openFile" => Some(&["path", "mode"]),
        "fs.openFileNoFollow" => Some(&["path", "mode"]),
        "fs.createTempFile" => Some(&["directory"]),
        "fs.tempDirectory" => Some(&[]),
        "fs.readLine" => Some(&["file"]),
        "fs.readAll" => Some(&["file"]),
        "fs.readAllBytes" => Some(&["file"]),
        "fs.writeAll" => Some(&["file", "value"]),
        "fs.writeAllBytes" => Some(&["file", "value"]),
        "fs.close" => Some(&["file"]),
        "fs.eof" => Some(&["file"]),
        "fs.canonicalPath" => Some(&["path"]),
        "fs.isWithin" => Some(&["path", "parent"]),
        "fs.pathJoin" => Some(&["parts"]),
        "fs.pathDirName" => Some(&["path"]),
        "fs.pathBaseName" => Some(&["path"]),
        "fs.pathExtension" => Some(&["path"]),
        "fs.pathNormalize" => Some(&["path"]),
        "fs.deleteFile" => Some(&["path"]),
        "fs.createDirectory" => Some(&["path"]),
        "fs.createDirectories" => Some(&["path"]),
        "fs.deleteDirectory" => Some(&["path"]),
        "fs.listDirectory" => Some(&["path"]),
        "fs.currentDirectory" => Some(&[]),
        "fs.setCurrentDirectory" => Some(&["path"]),
        "io.print" => Some(&["value"]),
        "io.write" => Some(&["value"]),
        "io.flush" => Some(&[]),
        "io.printError" => Some(&["value"]),
        "io.writeError" => Some(&["value"]),
        "io.flushError" => Some(&[]),
        "io.pollInput" => Some(&["timeoutMs"]),
        "json.parse" => Some(&["text"]),
        "json.stringify" => Some(&["value"]),
        "json.get" => Some(&["value", "key"]),
        "json.getOr" => Some(&["value", "key", "fallback"]),
        "thread.start" => Some(&["entry"]),
        "thread.isRunning" => Some(&["thread"]),
        "thread.waitFor" => Some(&["thread"]),
        "thread.cancel" => Some(&["thread"]),
        "thread.send" => Some(&["thread", "value"]),
        "thread.poll" => Some(&["thread"]),
        "thread.receive" => Some(&["thread"]),
        "thread.isCancelled" => Some(&["thread"]),
        "math.abs" => Some(&["value"]),
        "math.sign" => Some(&["value"]),
        "math.min" => Some(&["left", "right"]),
        "math.max" => Some(&["left", "right"]),
        "math.clamp" => Some(&["value", "minimum", "maximum"]),
        "math.floor" => Some(&["value"]),
        "math.ceil" => Some(&["value"]),
        "math.round" => Some(&["value"]),
        "math.trunc" => Some(&["value"]),
        "math.sqrt" => Some(&["value"]),
        "math.pow" => Some(&["value", "power"]),
        "math.exp" => Some(&["value"]),
        "math.log" => Some(&["value"]),
        "math.log10" => Some(&["value"]),
        "math.sin" => Some(&["value"]),
        "math.cos" => Some(&["value"]),
        "math.tan" => Some(&["value"]),
        "math.asin" => Some(&["value"]),
        "math.acos" => Some(&["value"]),
        "math.atan" => Some(&["value"]),
        "math.atan2" => Some(&["y", "x"]),
        "math.radians" => Some(&["value"]),
        "math.degrees" => Some(&["value"]),
        "math.isFinite" => Some(&["value"]),
        _ => None,
    }
}
