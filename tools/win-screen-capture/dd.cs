using System; using System.Runtime.InteropServices; using System.Drawing; using System.Drawing.Imaging;
public static class DD {
  [DllImport("d3d11.dll")] static extern int D3D11CreateDevice(IntPtr a, int dt, IntPtr sw, uint fl, IntPtr lv, uint n, uint sdk, out IntPtr dev, out int got, out IntPtr ctx);
  delegate int QI(IntPtr self, ref Guid iid, out IntPtr o);
  delegate uint Rel(IntPtr self);
  delegate int GetAdapter(IntPtr self, out IntPtr a);
  delegate int EnumOutputs(IntPtr self, uint i, out IntPtr o);
  delegate int DuplicateOutput(IntPtr self, IntPtr dev, out IntPtr dup);
  delegate int Acquire(IntPtr self, uint ms, IntPtr info, out IntPtr res);
  delegate int ReleaseFrame(IntPtr self);
  delegate void GetDesc(IntPtr self, IntPtr desc);
  delegate int CreateTex(IntPtr self, IntPtr desc, IntPtr init, out IntPtr tex);
  delegate int Map(IntPtr self, IntPtr res, uint sub, int type, uint flags, IntPtr mapped);
  delegate void Unmap(IntPtr self, IntPtr res, uint sub);
  delegate void Copy(IntPtr self, IntPtr dst, IntPtr src);
  static T F<T>(IntPtr o, int slot) { IntPtr vt = Marshal.ReadIntPtr(o); return (T)(object)Marshal.GetDelegateForFunctionPointer(Marshal.ReadIntPtr(vt, slot * IntPtr.Size), typeof(T)); }
  static IntPtr Q(IntPtr o, string g) { Guid id = new Guid(g); IntPtr r; int hr = F<QI>(o, 0)(o, ref id, out r); if (hr != 0) throw new Exception("QI " + hr.ToString("X")); return r; }
  static void Ck(int hr, string what) { if (hr != 0) throw new Exception(what + " " + hr.ToString("X")); }
  public static void Shot(string path) {
    IntPtr dev, ctx; int got;
    Ck(D3D11CreateDevice(IntPtr.Zero, 1, IntPtr.Zero, 0x20, IntPtr.Zero, 0, 7, out dev, out got, out ctx), "D3D11CreateDevice");
    IntPtr dxgiDev = Q(dev, "54ec77fa-1377-44e6-8c32-88fd5f44c84c");
    IntPtr adapter; Ck(F<GetAdapter>(dxgiDev, 7)(dxgiDev, out adapter), "GetAdapter");
    IntPtr output; Ck(F<EnumOutputs>(adapter, 7)(adapter, 0, out output), "EnumOutputs");
    IntPtr output1 = Q(output, "00cddea8-939b-4b83-a340-a685226666cc");
    IntPtr dup; Ck(F<DuplicateOutput>(output1, 22)(output1, dev, out dup), "DuplicateOutput");
    IntPtr info = Marshal.AllocHGlobal(64);
    IntPtr res = IntPtr.Zero;
    // Keep acquiring until a frame carries a desktop image (LastPresentTime != 0);
    // the first frames after DuplicateOutput can be empty.
    for (int i = 0; i < 20 && res == IntPtr.Zero; i++) {
      IntPtr r;
      int hr = F<Acquire>(dup, 8)(dup, 250, info, out r);
      if (hr != 0) continue;
      if (Marshal.ReadInt64(info, 0) != 0) { res = r; break; }
      F<Rel>(r, 2)(r); F<ReleaseFrame>(dup, 14)(dup);
    }
    if (res == IntPtr.Zero) { F<Rel>(dup, 2)(dup); throw new Exception("no desktop image"); }
    IntPtr tex = Q(res, "6f15aaf2-d208-4e89-9ab4-489535d34f9c");
    IntPtr desc = Marshal.AllocHGlobal(44);
    F<GetDesc>(tex, 10)(tex, desc);
    int w = Marshal.ReadInt32(desc, 0), h = Marshal.ReadInt32(desc, 4);
    Marshal.WriteInt32(desc, 8, 1); Marshal.WriteInt32(desc, 12, 1);
    Marshal.WriteInt32(desc, 28, 3); Marshal.WriteInt32(desc, 32, 0);
    Marshal.WriteInt32(desc, 36, 0x20000); Marshal.WriteInt32(desc, 40, 0);
    IntPtr staging; Ck(F<CreateTex>(dev, 5)(dev, desc, IntPtr.Zero, out staging), "CreateTexture2D");
    F<Copy>(ctx, 47)(ctx, staging, tex);
    IntPtr mapped = Marshal.AllocHGlobal(16);
    Ck(F<Map>(ctx, 14)(ctx, staging, 0, 1, 0, mapped), "Map");
    IntPtr data = Marshal.ReadIntPtr(mapped, 0); int pitch = Marshal.ReadInt32(mapped, 8);
    Bitmap bmp = new Bitmap(w, h, PixelFormat.Format32bppRgb);
    BitmapData bd = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppRgb);
    byte[] row = new byte[w * 4];
    for (int y = 0; y < h; y++) { Marshal.Copy(data + y * pitch, row, 0, w * 4); Marshal.Copy(row, 0, bd.Scan0 + y * bd.Stride, w * 4); }
    bmp.UnlockBits(bd);
    F<Unmap>(ctx, 15)(ctx, staging, 0);
    bmp.Save(path);
    bmp.Dispose();
    F<Rel>(staging, 2)(staging);
    F<Rel>(tex, 2)(tex);
    F<Rel>(res, 2)(res); F<ReleaseFrame>(dup, 14)(dup);
    F<Rel>(dup, 2)(dup);
    foreach (IntPtr o in new IntPtr[] { output1, output, adapter, dxgiDev, ctx, dev }) F<Rel>(o, 2)(o);
    Marshal.FreeHGlobal(info); Marshal.FreeHGlobal(desc); Marshal.FreeHGlobal(mapped);
  }
}
