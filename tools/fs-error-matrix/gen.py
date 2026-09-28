#!/usr/bin/env python3
"""Generate the fs error-matrix probe (bug-694, bug-701): one `AS Nothing` FUNC per case, each
TRAPping one `fs` call against the fixtures `fixtures.sh` / `fixtures.ps1` build,
and printing `e.code`. Usage: gen.py <out main.mfb>. See README.md."""
import sys
F="fx"
LONG="n"*300
cases=[
 # label, kind(let/res/sub), expr
 ("readText missing","let",f'fs::readText("{F}/nope.txt")'),
 ("readText empty","let",'fs::readText("")'),
 ("readText nul","let",f'fs::readText("{F}/a\\0b")'),
 ("readText dir","let",f'fs::readText("{F}/full")'),
 ("readText noperm","let",f'fs::readText("{F}/noread.txt")'),
 ("readText badutf8","let",f'fs::readText("{F}/bad.txt")'),
 ("readText notdir","let",f'fs::readText("{F}/plain.txt/x")'),
 ("readText loop","let",f'fs::readText("{F}/loopa")'),
 ("readText toolong","let",f'fs::readText("{F}/{LONG}")'),
 ("writeText toolong","sub",f'fs::writeText("{F}/{LONG}", "x")'),
 ("deleteDirectory toolong","sub",f'fs::deleteDirectory("{F}/{LONG}")'),
 ("readBytes missing","let",f'fs::readBytes("{F}/nope.txt")'),
 ("readBytes dir","let",f'fs::readBytes("{F}/full")'),
 ("readBytes noperm","let",f'fs::readBytes("{F}/noread.txt")'),
 ("writeText missingdir","sub",f'fs::writeText("{F}/nodir/x.txt", "x")'),
 ("writeText isdir","sub",f'fs::writeText("{F}/full", "x")'),
 ("writeText ro","sub",f'fs::writeText("{F}/ro/x.txt", "x")'),
 ("writeText empty","sub",'fs::writeText("", "x")'),
 ("writeText notdir","sub",f'fs::writeText("{F}/plain.txt/x", "x")'),
 ("writeText noperm","sub",f'fs::writeText("{F}/noread.txt", "x")'),
 ("appendText missingdir","sub",f'fs::appendText("{F}/nodir/x.txt", "x")'),
 ("appendBytes ro","sub",f'fs::appendBytes("{F}/ro/x.bin", BYTES)'),
 ("writeBytes isdir","sub",f'fs::writeBytes("{F}/full", BYTES)'),
 ("writeTextAtomic missingdir","sub",f'fs::writeTextAtomic("{F}/nodir/x.txt", "x")'),
 ("writeTextAtomic ro","sub",f'fs::writeTextAtomic("{F}/ro/x.txt", "x")'),
 ("writeTextAtomic empty","sub",'fs::writeTextAtomic("", "x")'),
 ("writeBytesAtomic isdir","sub",f'fs::writeBytesAtomic("{F}/full", BYTES)'),
 ("createTempFile missingdir","res",f'fs::createTempFile("{F}/nodir")'),
 ("createTempFile ro","res",f'fs::createTempFile("{F}/ro")'),
 ("createTempFile empty","res",'fs::createTempFile("")'),
 ("openFile badmode","res",f'fs::openFile("{F}/plain.txt", "bogus")'),
 ("openFile missing r","res",f'fs::openFile("{F}/nope.txt", "r")'),
 ("openFile missingdir w","res",f'fs::openFile("{F}/nodir/x", "w")'),
 ("openFile loop","res",f'fs::openFile("{F}/loopa", "r")'),
 ("openFile empty","res",'fs::openFile("", "r")'),
 ("openFile noperm","res",f'fs::openFile("{F}/noread.txt", "r")'),
 ("openFile isdir w","res",f'fs::openFile("{F}/full", "w")'),
 ("openFile isdir r","res",f'fs::openFile("{F}/full", "r")'),
 ("open missing","res",f'fs::open("{F}/nope.txt", "r")'),
 ("openFileNoFollow symlink","res",f'fs::openFileNoFollow("{F}/link.txt", "r")'),
 ("openFileNoFollow loop","res",f'fs::openFileNoFollow("{F}/loopa", "r")'),
 ("openWithin dotdot","res",f'fs::openWithin("{F}", "../x", "r")'),
 ("openWithin abs","res",f'fs::openWithin("{F}", "/etc/hosts", "r")'),
 ("openWithin escape","res",f'fs::openWithin("{F}", "link.txt", "r")'),
 ("openWithin missingroot","res",f'fs::openWithin("{F}/nodir", "x", "r")'),
 ("openWithin missing","res",f'fs::openWithin("{F}", "nope.txt", "r")'),
 ("openWithin emptyrel","res",f'fs::openWithin("{F}", "", "r")'),
 ("deleteFile dir","sub",f'fs::deleteFile("{F}/full")'),
 ("deleteFile missing","sub",f'fs::deleteFile("{F}/nope.txt")'),
 ("deleteFile ro","sub",f'fs::deleteFile("{F}/ro/x")'),
 ("deleteFile notdir","sub",f'fs::deleteFile("{F}/plain.txt/x")'),
 ("deleteDirectory nonempty","sub",f'fs::deleteDirectory("{F}/full")'),
 ("deleteDirectory file","sub",f'fs::deleteDirectory("{F}/plain.txt")'),
 ("deleteDirectory missing","sub",f'fs::deleteDirectory("{F}/nodir")'),
 ("createDirectory exists","sub",f'fs::createDirectory("{F}/full")'),
 ("createDirectory missingparent","sub",f'fs::createDirectory("{F}/nodir/x")'),
 ("createDirectory ro","sub",f'fs::createDirectory("{F}/ro/x")'),
 ("createDirectory notdir","sub",f'fs::createDirectory("{F}/plain.txt/x")'),
 ("createDirectories underfile","sub",f'fs::createDirectories("{F}/plain.txt/x/y")'),
 ("createDirectories ro","sub",f'fs::createDirectories("{F}/ro/x/y")'),
 ("createDirectories fileexists","sub",f'fs::createDirectories("{F}/plain.txt")'),
 ("createDirectories empty","sub",'fs::createDirectories("")'),
 ("listDirectory missing","let",f'fs::listDirectory("{F}/nodir")'),
 ("listDirectory file","let",f'fs::listDirectory("{F}/plain.txt")'),
 ("listDirectory ro-noexec","let",f'fs::listDirectory("{F}/noread.txt")'),
 ("canonicalPath missing","let",f'fs::canonicalPath("{F}/nope.txt")'),
 ("canonicalPath loop","let",f'fs::canonicalPath("{F}/loopa")'),
 ("canonicalPath notdir","let",f'fs::canonicalPath("{F}/plain.txt/x")'),
 ("canonicalPath empty","let",'fs::canonicalPath("")'),
 ("isWithin missing","let",f'fs::isWithin("{F}", "{F}/nope.txt")'),
 ("isWithin empty","let",f'fs::isWithin("", "{F}")'),
 ("exists nul","let",'fs::exists("a\\0b")'),
 ("exists empty","let",'fs::exists("")'),
 ("fileExists nul","let",'fs::fileExists("a\\0b")'),
 ("directoryExists nul","let",'fs::directoryExists("a\\0b")'),
 ("setCurrentDirectory missing","sub",f'fs::setCurrentDirectory("{F}/nodir")'),
 ("setCurrentDirectory file","sub",f'fs::setCurrentDirectory("{F}/plain.txt")'),
 ("setCurrentDirectory noperm","sub",f'fs::setCurrentDirectory("{F}/ro/../noread.txt")'),
 ("setCurrentDirectory empty","sub",'fs::setCurrentDirectory("")'),
 ("canonicalPath dir value","show",f'fs::canonicalPath("{F}/full")'),
 ("isWithin child value","show",f'toString(fs::isWithin("{F}", "{F}/full/f"))'),
 ("isWithin sibling value","show",f'toString(fs::isWithin("{F}/full", "{F}/ro"))'),
 ("canonicalPath link value","show",f'fs::canonicalPath("{F}/link.txt")'),
 ("setCurrentDirectory nox","sub",f'fs::setCurrentDirectory("{F}/nox")'),
 ("deleteFile in ro","sub",f'fs::deleteFile("{F}/ro/inside.txt")'),
 ("deleteDirectory in ro","sub",f'fs::deleteDirectory("{F}/ro/emptydir")'),
 ("listDirectory nox","let",f'fs::listDirectory("{F}/nox")'),
 ("canonicalPath nox child","let",f'fs::canonicalPath("{F}/nox/child")'),
 ("fileExists empty","show",'toString(fs::fileExists(""))'),
 ("directoryExists empty","show",'toString(fs::directoryExists(""))'),
 ("readLine eof","body",["RES h = fs::openFile(\"FX/lines.txt\", \"r\")", "LET a = fs::readLine(h)", "LET b = fs::readLine(h)"], 'fs::readLine(h)'),
 ("readLine badutf8","body",["RES h = fs::openFile(\"FX/bad.txt\", \"r\")"], 'fs::readLine(h)'),
 ("readAll badutf8","body",["RES h = fs::openFile(\"FX/bad.txt\", \"r\")"], 'fs::readAll(h)'),
 ("readBytesAt negative","body",["RES h = fs::openFile(\"FX/lines.txt\", \"r\")"], 'fs::readBytesAt(h, -1, 2)'),
 ("writeAll readonly handle","bodysub",["RES h = fs::openFile(\"FX/plain.txt\", \"r\")"], 'fs::writeAll(h, "x")'),
 ("setBuffered off drain fails","bodysub",["RES h = fs::openFile(\"FX/plain.txt\", \"r\")", "fs::setBuffered(h, TRUE)", "fs::writeAll(h, \"x\")"], 'fs::setBuffered(h, FALSE)'),
 ("eof closed","body",["RES h = fs::openFile(\"FX/lines.txt\", \"r\")", "closeIt(h)"], 'fs::eof(h)'),
 ("readLine closed","body",["RES h = fs::openFile(\"FX/lines.txt\", \"r\")", "closeIt(h)"], 'fs::readLine(h)'),
 ("close twice","bodysub",["RES h = fs::openFile(\"FX/lines.txt\", \"r\")", "closeIt(h)"], 'fs::close(h)'),
]
out=["IMPORT fs","IMPORT io","","FUNC closeIt(RES x AS fs::File) AS Nothing","  fs::close(x)","END FUNC",""]
names=[]
for i,case in enumerate(cases):
    n=f"c{i}"; names.append(n)
    out.append(f"FUNC {n}() AS Nothing")
    if len(case) == 4:
        label, kind, pre, expr = case
        for line in pre: out.append("  " + line.replace("FX", F))
        kind = "let" if kind == "body" else "sub"
    else:
        label, kind, expr = case
    if "BYTES" in expr: out.append("  LET bs AS List OF Byte = [1, 2]")
    if kind in ("let", "show"): out.append(f"  LET v = {expr} TRAP(e)")
    elif False: pass
    elif kind=="res": out.append(f"  RES v = {expr} TRAP(e)")
    else: out.append(f"  {expr.replace('BYTES','bs')} TRAP(e)")
    out.append(f'    io::print("{label}: " & toString(e.code))')
    out.append("    RETURN")
    out.append("  END TRAP")
    if kind == "show": out.append(f'  io::print("{label}: " & v)')
    else: out.append(f'  io::print("{label}: ok")')
    out.append("END FUNC")
    out.append("")
out.append("FUNC main() AS Nothing")
for n in names: out.append(f"  {n}()")
out.append("END FUNC")
open(sys.argv[1],"w").write("\n".join(out)+"\n")
