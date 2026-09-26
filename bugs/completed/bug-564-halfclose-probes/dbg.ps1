param([string]$exe)
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class MiniDbg {
  [StructLayout(LayoutKind.Sequential)] public struct STARTUPINFO { public int cb; public IntPtr a,b,c; public int d,e,f,g,h,i,j,k; public short l,m; public IntPtr n,o,p,q; }
  [StructLayout(LayoutKind.Sequential)] public struct PI { public IntPtr hProcess, hThread; public int pid, tid; }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateProcessW(string app, string cmd, IntPtr pa, IntPtr ta, bool inh, int flags, IntPtr env, string cwd, ref STARTUPINFO si, out PI pi);
  [DllImport("kernel32.dll")] static extern bool WaitForDebugEvent(byte[] ev, int ms);
  [DllImport("kernel32.dll")] static extern bool ContinueDebugEvent(int pid, int tid, int status);
  [DllImport("kernel32.dll")] static extern IntPtr OpenThread(int access, bool inh, int tid);
  [DllImport("kernel32.dll")] static extern bool GetThreadContext(IntPtr h, IntPtr ctx);
  [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr hp, IntPtr addr, byte[] buf, int n, out IntPtr read);
  public static string Run(string exe) {
    var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(si); PI pi;
    if (!CreateProcessW(exe, null, IntPtr.Zero, IntPtr.Zero, false, 2, IntPtr.Zero, null, ref si, out pi)) return "create failed " + Marshal.GetLastWin32Error();
    byte[] ev = new byte[256]; IntPtr imageBase = IntPtr.Zero; bool first = true;
    while (WaitForDebugEvent(ev, 60000)) {
      int code = BitConverter.ToInt32(ev, 0), pid = BitConverter.ToInt32(ev, 4), tid = BitConverter.ToInt32(ev, 8);
      int cont = 0x00010002; // DBG_CONTINUE
      if (code == 3) imageBase = (IntPtr)BitConverter.ToInt64(ev, 12 + 4 + 8 + 8 + 8); // CREATE_PROCESS: hFile,hProcess,hThread,lpBaseOfImage
      if (code == 1) { // EXCEPTION
        uint exc = BitConverter.ToUInt32(ev, 16); long addr = BitConverter.ToInt64(ev, 16 + 16);
        if (exc == 0x80000003 && first) { first = false; }
        else if (exc == 0xC0000005) {
          IntPtr mem = Marshal.AllocHGlobal(1232 + 16); IntPtr ctx = (IntPtr)(((long)mem + 15) & ~15L);
          for (int i = 0; i < 1232; i++) Marshal.WriteByte(ctx, i, 0);
          Marshal.WriteInt32(ctx, 0x30, 0x10000B); // CONTEXT_FULL (AMD64)
          IntPtr th = OpenThread(0x1FFFFF, false, tid); GetThreadContext(th, ctx);
          var sb = new StringBuilder(); sb.AppendFormat("AV at 0x{0:X} base 0x{1:X} rva 0x{2:X}\n", addr, (long)imageBase, addr - (long)imageBase);
          string[] names = {"rax","rcx","rdx","rbx","rsp","rbp","rsi","rdi","r8","r9","r10","r11","r12","r13","r14","r15"};
          for (int i = 0; i < 16; i++) sb.AppendFormat("{0}=0x{1:X} ", names[i], Marshal.ReadInt64(ctx, 0x78 + i * 8));
          long rsp = Marshal.ReadInt64(ctx, 0x78 + 4 * 8); byte[] stk = new byte[8 * 64]; IntPtr rd;
          ReadProcessMemory(pi.hProcess, (IntPtr)rsp, stk, stk.Length, out rd);
          sb.Append("\nstack words that look like image RVAs:");
          for (int i = 0; i < 64; i++) { long w = BitConverter.ToInt64(stk, i * 8); long r = w - (long)imageBase; if (r > 0x1000 && r < 0x100000) sb.AppendFormat(" [+{0}]0x{1:X}", i * 8, r); }
          return sb.ToString();
        } else cont = unchecked((int)0x80010001); // DBG_EXCEPTION_NOT_HANDLED
      }
      if (code == 5) return "exited without AV";
      ContinueDebugEvent(pid, tid, cont);
    }
    return "timeout";
  }
}
"@
[MiniDbg]::Run($exe)
