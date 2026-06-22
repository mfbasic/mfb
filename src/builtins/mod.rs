pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod math;
pub(crate) mod net;
pub(crate) mod resource;
pub(crate) mod strings;
pub(crate) mod thread;

pub(crate) use resource::{ResourceInfo, ResourceKind, ResourceRegistry};

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(
        name,
        "fs" | "io" | "json" | "math" | "net" | "strings" | "thread"
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    fs::is_builtin_type(name)
        || io::is_builtin_type(name)
        || json::is_builtin_type(name)
        || net::is_builtin_type(name)
        || thread::is_builtin_type(name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    resource::builtin_resource_close_function(type_name)
}

pub(crate) fn is_resource_type(type_name: &str) -> bool {
    resource::is_builtin_resource_type(type_name)
}

pub(crate) fn is_thread_sendable_resource_type(type_name: &str) -> bool {
    resource::is_builtin_sendable_resource_type(type_name)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    general::call_return_type_name(name)
        .or_else(|| strings::call_return_type_name(name))
        .or_else(|| math::call_return_type_name(name))
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
        .or_else(|| json::call_return_type_name(name))
        .or_else(|| net::call_return_type_name(name))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    general::is_general_call(name)
        || strings::is_strings_call(name)
        || math::is_math_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || json::is_json_call(name)
        || net::is_net_call(name)
        || thread::is_thread_call(name)
        || call_return_type_name(name).is_some()
}

pub(crate) fn is_builtin_member(name: &str) -> bool {
    is_builtin_call(name) || math::is_math_constant(name)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        "error" => Some(&[&["code"], &["message"]]),
        "len" => Some(&[&["value"]]),
        "find" => Some(&[&["value"], &["needle", "item"], &["start"]]),
        "mid" => Some(&[&["value"], &["start"], &["count"]]),
        "replace" => Some(&[&["value"], &["old", "needle"], &["new", "replacement"]]),
        "typeName" => Some(&[&["value"]]),
        "toString" => Some(&[&["value"], &["precision", "decimals"]]),
        "toInt" => Some(&[&["value"]]),
        "toFloat" => Some(&[&["value"]]),
        "toFixed" => Some(&[&["value"]]),
        "toByte" => Some(&[&["value"]]),
        "isNumeric" => Some(&[&["value"]]),
        "isEven" => Some(&[&["value"]]),
        "isOdd" => Some(&[&["value"]]),
        "isPositive" => Some(&[&["value"]]),
        "isNegative" => Some(&[&["value"]]),
        "isZero" => Some(&[&["value"]]),
        "isEmpty" => Some(&[&["value"]]),
        "isNotEmpty" => Some(&[&["value"]]),
        "get" => Some(&[&["value", "collection"], &["index", "key"]]),
        "getOr" => Some(&[
            &["value", "collection"],
            &["index", "key"],
            &["default", "fallback"],
        ]),
        "set" => Some(&[&["value", "collection"], &["index", "key"], &["item"]]),
        "append" => Some(&[&["value", "list"], &["item", "items"]]),
        "prepend" => Some(&[&["value", "list"], &["item"]]),
        "insert" => Some(&[&["value", "list"], &["index"], &["item"]]),
        "removeAt" => Some(&[&["value", "list"], &["index"]]),
        "removeKey" => Some(&[&["value", "map"], &["key"]]),
        "keys" => Some(&[&["value", "map"]]),
        "values" => Some(&[&["value", "map"]]),
        "hasKey" => Some(&[&["value", "map"], &["key"]]),
        "contains" => Some(&[&["value", "collection"], &["item"]]),
        "forEach" => Some(&[&["value", "collection"], &["action"]]),
        "transform" => Some(&[&["value", "collection"], &["f", "transform"]]),
        "filter" => Some(&[&["value", "collection"], &["predicate"]]),
        "reduce" => Some(&[
            &["value", "collection"],
            &["initial", "seed"],
            &["f", "combine"],
        ]),
        "sum" => Some(&[&["value", "collection"]]),
        "strings.trim" => Some(&[&["value"]]),
        "strings.trimStart" => Some(&[&["value"]]),
        "strings.trimEnd" => Some(&[&["value"]]),
        "strings.upper" => Some(&[&["value"]]),
        "strings.lower" => Some(&[&["value"]]),
        "strings.caseFold" => Some(&[&["value"]]),
        "strings.normalizeNfc" => Some(&[&["value"]]),
        "strings.graphemes" => Some(&[&["value"]]),
        "strings.startsWith" => Some(&[&["value"], &["prefix"]]),
        "strings.endsWith" => Some(&[&["value"], &["suffix"]]),
        "strings.contains" => Some(&[&["value"], &["needle"]]),
        "strings.split" => Some(&[&["value"], &["delimiter", "separator"]]),
        "strings.join" => Some(&[&["parts", "values"], &["delimiter", "separator"]]),
        "strings.byteLen" => Some(&[&["value"]]),
        "fs.fileExists" => Some(&[&["path"]]),
        "fs.directoryExists" => Some(&[&["path"]]),
        "fs.exists" => Some(&[&["path"]]),
        "fs.readBytes" => Some(&[&["path"]]),
        "fs.readText" => Some(&[&["path"]]),
        "fs.writeBytes" => Some(&[&["path"], &["bytes", "value"]]),
        "fs.writeText" => Some(&[&["path"], &["value"]]),
        "fs.writeBytesAtomic" => Some(&[&["path"], &["bytes", "value"]]),
        "fs.writeTextAtomic" => Some(&[&["path"], &["value"]]),
        "fs.appendBytes" => Some(&[&["path"], &["bytes", "value"]]),
        "fs.appendText" => Some(&[&["path"], &["value"]]),
        "fs.open" => Some(&[&["path"], &["mode"]]),
        "fs.openFile" => Some(&[&["path"], &["mode"]]),
        "fs.openFileNoFollow" => Some(&[&["path"], &["mode"]]),
        "fs.createTempFile" => Some(&[&["directory"]]),
        "fs.tempDirectory" => Some(&[]),
        "fs.readLine" => Some(&[&["file"]]),
        "fs.readAll" => Some(&[&["file"]]),
        "fs.readAllBytes" => Some(&[&["file"]]),
        "fs.writeAll" => Some(&[&["file"], &["value"]]),
        "fs.writeAllBytes" => Some(&[&["file"], &["bytes", "value"]]),
        "fs.close" => Some(&[&["file"]]),
        "fs.eof" => Some(&[&["file"]]),
        "fs.canonicalPath" => Some(&[&["path"]]),
        "fs.isWithin" => Some(&[&["base", "path"], &["child", "parent"]]),
        "fs.pathJoin" => Some(&[&["parts"]]),
        "fs.pathDirName" => Some(&[&["path"]]),
        "fs.pathBaseName" => Some(&[&["path"]]),
        "fs.pathExtension" => Some(&[&["path"]]),
        "fs.pathNormalize" => Some(&[&["path"]]),
        "fs.deleteFile" => Some(&[&["path"]]),
        "fs.createDirectory" => Some(&[&["path"]]),
        "fs.createDirectories" => Some(&[&["path"]]),
        "fs.deleteDirectory" => Some(&[&["path"]]),
        "fs.listDirectory" => Some(&[&["path"]]),
        "fs.currentDirectory" => Some(&[]),
        "fs.setCurrentDirectory" => Some(&[&["path"]]),
        "net.lookup" => Some(&[&["host"], &["port"]]),
        "net.connectTcp" => Some(&[&["host", "address"], &["port", "timeoutMs"], &["timeoutMs"]]),
        "net.listenTcp" => Some(&[&["host"], &["port"], &["backlog"]]),
        "net.accept" => Some(&[&["listener"], &["timeoutMs"]]),
        "net.poll" => Some(&[&["sock"], &["timeoutMs"]]),
        "net.read" => Some(&[&["sock"], &["maxBytes"]]),
        "net.readText" => Some(&[&["sock"], &["maxBytes"]]),
        "net.write" => Some(&[&["sock"], &["bytes"]]),
        "net.writeText" => Some(&[&["sock"], &["value"]]),
        "net.close" => Some(&[&["resource", "sock", "listener"]]),
        "net.localAddress" => Some(&[&["sock", "listener"]]),
        "net.remoteAddress" => Some(&[&["sock"]]),
        "net.setReadTimeout" => Some(&[&["sock"], &["timeoutMs"]]),
        "net.setWriteTimeout" => Some(&[&["sock"], &["timeoutMs"]]),
        "net.bindUdp" => Some(&[&["host"], &["port"]]),
        "net.receiveFrom" => Some(&[&["sock"], &["maxBytes"]]),
        "net.receiveTextFrom" => Some(&[&["sock"], &["maxBytes"]]),
        "net.sendTo" => Some(&[&["sock"], &["address"], &["bytes"]]),
        "net.sendTextTo" => Some(&[&["sock"], &["address"], &["value"]]),
        "io.print" => Some(&[&["value"]]),
        "io.write" => Some(&[&["value"]]),
        "io.flush" => Some(&[]),
        "io.printError" => Some(&[&["value"]]),
        "io.writeError" => Some(&[&["value"]]),
        "io.flushError" => Some(&[]),
        "io.input" => Some(&[&["prompt"]]),
        "io.readLine" => Some(&[]),
        "io.readChar" => Some(&[]),
        "io.readByte" => Some(&[]),
        "io.pollInput" => Some(&[&["timeoutMs"]]),
        "io.isInputTerminal" => Some(&[]),
        "io.isOutputTerminal" => Some(&[]),
        "io.isErrorTerminal" => Some(&[]),
        "io.terminalSize" => Some(&[]),
        "json.parse" => Some(&[&["value", "text"]]),
        "json.stringify" => Some(&[&["value"]]),
        "json.get" => Some(&[&["value"], &["path", "key"]]),
        "json.getOr" => Some(&[
            &["value"],
            &["path", "key"],
            &["default", "defaultValue", "fallback"],
        ]),
        "thread.start" => Some(&[
            &["f", "entry"],
            &["data"],
            &["inboundLimit"],
            &["outboundLimit"],
        ]),
        "thread.isRunning" => Some(&[&["t", "thread"]]),
        "thread.waitFor" => Some(&[&["t", "thread"]]),
        "thread.cancel" => Some(&[&["t", "thread"]]),
        "thread.send" => Some(&[&["t", "thread"], &["data", "value"], &["timeoutMs"]]),
        "thread.poll" => Some(&[&["t", "thread"], &["ms"]]),
        "thread.receive" => Some(&[&["t", "thread"], &["timeoutMs"]]),
        "thread.isCancelled" => Some(&[&["t", "thread"]]),
        "math.abs" => Some(&[&["value"]]),
        "math.min" => Some(&[&["a", "left"], &["b", "right"]]),
        "math.max" => Some(&[&["a", "left"], &["b", "right"]]),
        "math.clamp" => Some(&[&["value"], &["low", "minimum"], &["high", "maximum"]]),
        "math.floor" => Some(&[&["value"]]),
        "math.ceil" => Some(&[&["value"]]),
        "math.round" => Some(&[&["value"]]),
        "math.sqrt" => Some(&[&["value"]]),
        "math.pow" => Some(&[&["base", "value"], &["exponent", "power"]]),
        "math.exp" => Some(&[&["value"]]),
        "math.log" => Some(&[&["value"]]),
        "math.log10" => Some(&[&["value"]]),
        "math.sin" => Some(&[&["value"]]),
        "math.cos" => Some(&[&["value"]]),
        "math.tan" => Some(&[&["value"]]),
        "math.asin" => Some(&[&["value"]]),
        "math.acos" => Some(&[&["value"]]),
        "math.atan" => Some(&[&["value"]]),
        "math.atan2" => Some(&[&["y"], &["x"]]),
        _ => None,
    }
}
