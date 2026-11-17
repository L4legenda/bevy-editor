// Standalone CTE-450 user-mode driver. Windows HID -> Windows Ink.
// Protocol constants cross-checked against OpenTabletDriver's CTE-450 configuration:
// https://github.com/OpenTabletDriver/OpenTabletDriver/tree/master/OpenTabletDriver.Configurations
// Windows ABI and injection rules: Microsoft Windows SDK winuser.h / hidpi.h.
using System;
using System.ComponentModel;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32.SafeHandles;

namespace BambooDriver {
    internal struct Report {
        internal int X, Y, Pressure;
        internal bool InRange, Contact, Eraser, Barrel;
        internal static bool Parse(byte[] b, out Report r) {
            r = new Report();
            // Report 2 also contains mouse and pad-button packets; neither is a pen.
            if (b.Length != 9 || b[0] != 2 || (b[1] & 0x40) != 0) return false;
            r.X = b[2] | b[3] << 8; r.Y = b[4] | b[5] << 8;
            r.InRange = (b[1] & 0x80) != 0;
            if (!r.InRange && r.X == 0 && r.Y == 0 && (b[6] | (b[7] & 3)) == 0) return false;
            r.Contact = r.InRange && (b[1] & 1) != 0;
            r.Pressure = r.Contact ? Math.Min(511, b[6] | (b[7] & 3) << 8) : 0;
            r.Eraser = (b[1] & 0x20) != 0; r.Barrel = (b[1] & 6) != 0;
            return r.X <= 14760 && r.Y <= 9225;
        }
        internal Point Map(Rectangle screen) {
            return new Point(screen.Left + (int)Math.Round(X * (screen.Width - 1.0) / 14760),
                screen.Top + (int)Math.Round(Y * (screen.Height - 1.0) / 9225));
        }
    }

    internal static class Native {
        [StructLayout(LayoutKind.Sequential)] internal struct Interface {
            internal int Size; internal Guid Class; internal int Flags; internal IntPtr Reserved;
        }
        [StructLayout(LayoutKind.Sequential)] internal struct PointerInfo {
            internal uint Type, Id, Frame, Flags;
            internal IntPtr Source, Target;
            internal Point Pixel, Himetric, PixelRaw, HimetricRaw;
            internal uint Time, History; internal int InputData; internal uint Keys;
            internal ulong Performance; internal int ButtonChange;
        }
        [StructLayout(LayoutKind.Sequential)] internal struct PenInfo {
            internal PointerInfo Pointer;
            internal uint Flags, Mask, Pressure, Rotation; internal int TiltX, TiltY;
        }
        [StructLayout(LayoutKind.Sequential)] internal struct TouchInfo {
            internal PointerInfo Pointer; internal uint Flags, Mask;
            internal Rectangle Contact, RawContact; internal uint Orientation, Pressure;
        }
        // On x64 the union begins at offset 8; touch is the largest member.
        [StructLayout(LayoutKind.Explicit, Size = 152)] internal struct PointerTypeInfo {
            [FieldOffset(0)] internal uint Type;
            [FieldOffset(8)] internal PenInfo Pen;
        }
        [DllImport("hid.dll")] internal static extern void HidD_GetHidGuid(out Guid guid);
        [DllImport("hid.dll", SetLastError=true)] internal static extern bool HidD_GetPreparsedData(SafeFileHandle file, out IntPtr data);
        [DllImport("hid.dll")] internal static extern bool HidD_FreePreparsedData(IntPtr data);
        [DllImport("hid.dll")] internal static extern int HidP_GetCaps(IntPtr data, IntPtr caps);
        [DllImport("hid.dll", SetLastError=true)] internal static extern bool HidD_GetFeature(SafeFileHandle file, byte[] data, int size);
        [DllImport("hid.dll", SetLastError=true)] internal static extern bool HidD_SetFeature(SafeFileHandle file, byte[] data, int size);
        [DllImport("setupapi.dll", CharSet=CharSet.Unicode)] internal static extern IntPtr SetupDiGetClassDevs(ref Guid guid, IntPtr enumerator, IntPtr parent, uint flags);
        [DllImport("setupapi.dll")] internal static extern bool SetupDiEnumDeviceInterfaces(IntPtr set, IntPtr device, ref Guid guid, uint index, ref Interface info);
        [DllImport("setupapi.dll", CharSet=CharSet.Unicode)] internal static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr set, ref Interface info, IntPtr detail, uint size, out uint needed, IntPtr device);
        [DllImport("setupapi.dll")] internal static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] internal static extern SafeFileHandle CreateFile(string name, uint access, uint sharing, IntPtr security, uint creation, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError=true)] internal static extern bool CancelIoEx(SafeFileHandle file, IntPtr overlapped);
        [DllImport("user32.dll", SetLastError=true)] internal static extern IntPtr CreateSyntheticPointerDevice(uint type, uint count, uint feedback);
        [DllImport("user32.dll")] internal static extern void DestroySyntheticPointerDevice(IntPtr device);
        [DllImport("user32.dll", SetLastError=true)] internal static extern bool InjectSyntheticPointerInput(IntPtr device, [In] PointerTypeInfo[] pointers, uint count);
        [DllImport("user32.dll", SetLastError=true)] internal static extern bool GetPointerPenInfo(uint id, out PenInfo pen);
        [DllImport("user32.dll", SetLastError=true)] internal static extern bool GetPointerDeviceRects(IntPtr source, out Rectangle device, out Rectangle display);
        [DllImport("user32.dll")] internal static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    }

    internal sealed class Tablet : IDisposable {
        private SafeFileHandle file;
        private FileStream stream;
        private byte[] originalMode;
        private readonly string path;
        private bool initialized;
        internal Tablet() {
            path = Find();
            if (path == null) throw new IOException("CTE-450 не найден или занят другим драйвером. Подключите планшет по USB.");
        }
        internal static string Find() {
            Guid guid; Native.HidD_GetHidGuid(out guid);
            IntPtr set = Native.SetupDiGetClassDevs(ref guid, IntPtr.Zero, IntPtr.Zero, 18);
            if (set == new IntPtr(-1)) throw new Win32Exception();
            try {
                for (uint index = 0; ; index++) {
                    Native.Interface info = new Native.Interface(); info.Size = Marshal.SizeOf(info);
                    if (!Native.SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref guid, index, ref info)) break;
                    IntPtr detail = Marshal.AllocHGlobal(4096);
                    try {
                        Marshal.WriteInt32(detail, 8); uint needed;
                        if (!Native.SetupDiGetDeviceInterfaceDetail(set, ref info, detail, 4096, out needed, IntPtr.Zero)) continue;
                        string name = Marshal.PtrToStringUni(IntPtr.Add(detail, 4));
                        if (name.IndexOf("vid_056a&pid_0017", StringComparison.OrdinalIgnoreCase) < 0) continue;
                        using (SafeFileHandle handle = Native.CreateFile(name, 0x80000000, 3, IntPtr.Zero, 3, 0x40000000, IntPtr.Zero)) {
                            if (handle.IsInvalid) continue;
                            IntPtr data;
                            if (!Native.HidD_GetPreparsedData(handle, out data)) continue;
                            try {
                                IntPtr caps = Marshal.AllocHGlobal(64);
                                try {
                                    if (Native.HidP_GetCaps(data, caps) >= 0 && Marshal.ReadInt16(caps, 2) == 13 && Marshal.ReadInt16(caps, 4) == 9) return name;
                                } finally { Marshal.FreeHGlobal(caps); }
                            } finally { Native.HidD_FreePreparsedData(data); }
                        }
                    } finally { Marshal.FreeHGlobal(detail); }
                }
                return null;
            } finally { Native.SetupDiDestroyDeviceInfoList(set); }
        }
        internal void Start() {
            file = Native.CreateFile(path, 0x80000000, 3, IntPtr.Zero, 3, 0x40000000, IntPtr.Zero);
            if (file.IsInvalid) throw new Win32Exception();
            // Read the current mode before changing it so Stop can restore the tablet.
            originalMode = new byte[] { 2, 0 };
            if (!Native.HidD_GetFeature(file, originalMode, originalMode.Length))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Не удалось прочитать исходный режим планшета");
            Program.Log("Original mode: " + BitConverter.ToString(originalMode));
            if (!Native.HidD_SetFeature(file, new byte[] { 2, 2 }, 2))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Не удалось включить режим пера");
            initialized = true;
            stream = new FileStream(file, FileAccess.Read, 9, true);
        }
        internal async Task<Report?> Read(CancellationToken token) {
            byte[] buffer = new byte[9];
            int count = await stream.ReadAsync(buffer, 0, buffer.Length, token).ConfigureAwait(false);
            if (count == 0) throw new IOException("Планшет отключён");
            Report report;
            return Report.Parse(count == 9 ? buffer : new byte[0], out report) ? (Report?)report : null;
        }
        internal void Cancel() { if (file != null && !file.IsInvalid && !file.IsClosed) Native.CancelIoEx(file, IntPtr.Zero); }
        public void Dispose() {
            if (file != null && !file.IsInvalid && !file.IsClosed && initialized) {
                if (!Native.HidD_SetFeature(file, originalMode, originalMode.Length))
                    Program.Log("Mode restore failed; reconnect tablet. Error=" + Marshal.GetLastWin32Error());
            }
            initialized = false;
            if (stream != null) stream.Dispose();
            if (file != null) file.Dispose();
        }
    }

    internal sealed class Pen : IDisposable {
        private IntPtr device;
        private bool down, inRange;
        private uint id = 1;
        private Point last;
        internal Pen() {
            device = Native.CreateSyntheticPointerDevice(3, 1, 3);
            if (device == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows Ink недоступен");
        }
        internal static Native.PointerTypeInfo Frame(Point point, uint id, uint flags, uint pressure, bool eraser, bool barrel) {
            Native.PointerTypeInfo frame = new Native.PointerTypeInfo();
            frame.Type = 3; frame.Pen.Pointer.Type = 3; frame.Pen.Pointer.Id = id;
            frame.Pen.Pointer.Pixel = point; frame.Pen.Pointer.PixelRaw = point;
            frame.Pen.Pointer.Flags = flags | 0x2000; // primary pointer
            frame.Pen.Mask = 1; frame.Pen.Pressure = pressure;
            frame.Pen.Flags = (eraser ? 6u : 0u) | (barrel ? 1u : 0u);
            return frame;
        }
        private void Send(Point point, uint flags, uint pressure, bool eraser, bool barrel) {
            Native.PointerTypeInfo frame = Frame(point, id, flags, pressure, eraser, barrel);
            if (!Native.InjectSyntheticPointerInput(device, new Native.PointerTypeInfo[] { frame }, 1))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Не удалось передать Windows Ink");
            last = point;
        }
        internal void Update(Report report, Rectangle screen) {
            Point point = report.Map(screen);
            if (!report.InRange) { Leave(); return; }
            bool contact = report.Contact;
            // Establish hover first, then contact. No timestamps: Windows supplies them.
            if (!inRange) { Send(point, 2 | 0x20000, 0, report.Eraser, report.Barrel); inRange = true; }
            uint flags = 2 | (contact ? 4u | 0x10u : 0u);
            flags |= contact && !down ? 0x10000u : !contact && down ? 0x40000u : 0x20000u;
            Send(point, flags, (uint)Math.Round(report.Pressure * 1024.0 / 511), report.Eraser, report.Barrel);
            down = contact;
        }
        private void Leave() {
            if (down) { Send(last, 2 | 0x40000, 0, false, false); down = false; }
            if (inRange) { Send(last, 0x20000, 0, false, false); inRange = false; id++; }
        }
        public void Dispose() {
            if (device == IntPtr.Zero) return;
            try { Leave(); } finally { Native.DestroySyntheticPointerDevice(device); device = IntPtr.Zero; }
        }
    }

    internal sealed class DriverForm : Form {
        private readonly Label status = new Label(), values = new Label();
        private readonly ProgressBar meter = new ProgressBar();
        private readonly ComboBox monitors = new ComboBox();
        private readonly Button start = new Button(), stop = new Button();
        private readonly CheckBox ink = new CheckBox();
        private readonly NotifyIcon tray = new NotifyIcon();
        private CancellationTokenSource cancellation;
        private Tablet tablet;
        private Task work;
        private volatile int pressure, packets, maximum;
        private string failure;
        private readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();
        private bool closing;
        private bool stopping;
        private bool showSettings, exitRequested;
        protected override void SetVisibleCore(bool value) {
            // Application.Run must own a window, but startup stays entirely in the tray.
            base.SetVisibleCore(value && showSettings);
        }
        private void OpenSettings() {
            showSettings = true; Show(); WindowState = FormWindowState.Normal; Activate();
        }
        private void HideSettings() { showSettings = false; Hide(); }
        internal DriverForm() {
            ShowInTaskbar = false;
            using (Stream resource = typeof(Program).Assembly.GetManifestResourceStream("BambooDriver.ico"))
            using (Icon loaded = new Icon(resource)) Icon = (Icon)loaded.Clone();
            Text = "Bamboo Fun CTE-450"; ClientSize = new Size(520, 340); MinimumSize = new Size(536, 379);
            BackColor = Color.FromArgb(22, 23, 28); ForeColor = Color.WhiteSmoke;
            Font = new Font("Segoe UI", 10); StartPosition = FormStartPosition.CenterScreen;
            Label title = new Label { Text = "Bamboo Fun CTE-450", Font = new Font("Segoe UI Semibold", 19), AutoSize = true, Location = new Point(24, 20) };
            status.SetBounds(24, 64, 470, 42); status.Text = "Нажмите «Запустить», затем проведите пером по планшету.";
            monitors.SetBounds(24, 116, 470, 28); monitors.DropDownStyle = ComboBoxStyle.DropDownList;
            foreach (Screen screen in Screen.AllScreens) monitors.Items.Add(screen.DeviceName + " — " + screen.Bounds.Width + " × " + screen.Bounds.Height);
            monitors.SelectedIndex = Array.FindIndex(Screen.AllScreens, delegate(Screen s) { return s.Primary; });
            ink.SetBounds(24, 158, 470, 26); ink.Text = "Передавать нажим в программы через Windows Ink"; ink.Checked = true;
            values.SetBounds(24, 198, 470, 26); values.Text = "Нажим: 0 / 511   ·   Пакетов: 0";
            meter.SetBounds(24, 232, 470, 14); meter.Maximum = 511;
            start.SetBounds(24, 274, 145, 36); start.Text = "Запустить";
            stop.SetBounds(181, 274, 145, 36); stop.Text = "Остановить"; stop.Enabled = false;
            Button hide = new Button { Text = "Свернуть в трей", Bounds = new Rectangle(338, 274, 156, 36) };
            foreach (Button b in new Button[] { start, stop, hide }) { b.FlatStyle = FlatStyle.Flat; b.BackColor = Color.FromArgb(42, 44, 53); }
            Controls.AddRange(new Control[] { title, status, monitors, ink, values, meter, start, stop, hide });
            start.Click += delegate { StartDriver(); }; stop.Click += async delegate { await StopDriver(); };
            tray.Icon = Icon; tray.Text = "Bamboo CTE-450";
            ContextMenuStrip menu = new ContextMenuStrip();
            menu.Items.Add("Открыть настройки", null, delegate { OpenSettings(); });
            menu.Items.Add("Остановить и выйти", null, delegate { exitRequested = true; Close(); }); tray.ContextMenuStrip = menu;
            tray.DoubleClick += delegate { OpenSettings(); };
            hide.Click += delegate { HideSettings(); };
            Resize += delegate { if (WindowState == FormWindowState.Minimized) HideSettings(); };
            timer.Interval = 50; timer.Tick += async delegate {
                meter.Value = Math.Max(0, Math.Min(511, pressure));
                values.Text = "Нажим: " + pressure + " / 511   ·   Максимум: " + maximum + "   ·   Пакетов: " + packets;
                if (work != null && work.IsCompleted && cancellation != null) {
                    string error = failure; await StopDriver();
                    status.Text = error == null ? "Остановлен" : error;
                    tray.Text = "Bamboo CTE-450 · Остановлен";
                    if (error != null) tray.ShowBalloonTip(4000, "Bamboo CTE-450", error, ToolTipIcon.Warning);
                }
            }; timer.Start();
            FormClosing += async delegate(object sender, FormClosingEventArgs e) {
                if (!exitRequested && e.CloseReason == CloseReason.UserClosing) { e.Cancel = true; HideSettings(); return; }
                if (closing) return; e.Cancel = true; Enabled = false;
                await StopDriver(); closing = true; tray.Dispose(); timer.Dispose(); Close();
            };
            tray.Visible = true;
            // Create the message target before starting the asynchronous HID reader.
            IntPtr messageTarget = Handle;
            BeginInvoke((Action)StartDriver);
        }
        private void StartDriver() {
            if (work != null) return;
            Rectangle screen = Screen.AllScreens[monitors.SelectedIndex].Bounds;
            bool output = ink.Checked;
            cancellation = new CancellationTokenSource(); failure = null; packets = pressure = maximum = 0;
            CancellationToken token = cancellation.Token;
            start.Enabled = monitors.Enabled = ink.Enabled = false; stop.Enabled = true;
            status.Text = "Подключение…";
            work = Task.Run(async delegate {
                try {
                    using (Tablet connected = new Tablet()) {
                        tablet = connected;
                        using (Pen pen = output ? new Pen() : null) {
                            connected.Start();
                            BeginInvoke((Action)delegate {
                                status.Text = output ? "Работает · Windows Ink" : "Проверка нажима · без передачи в программы";
                                tray.Text = "Bamboo CTE-450 · Работает";
                            });
                            while (!token.IsCancellationRequested) {
                                Report? report = await connected.Read(token).ConfigureAwait(false);
                                if (!report.HasValue) continue;
                                pressure = report.Value.Pressure; maximum = Math.Max(maximum, pressure); packets++;
                                if (pen != null) pen.Update(report.Value, screen);
                            }
                        }
                    }
                } catch (OperationCanceledException) { }
                catch (Exception error) { if (!token.IsCancellationRequested) { failure = error.Message; Program.Log(error.ToString()); } }
                finally { tablet = null; pressure = 0; }
            });
        }
        private async Task StopDriver() {
            if (cancellation == null) return;
            if (stopping) { if (work != null) await work; return; }
            stopping = true; timer.Stop(); stop.Enabled = false;
            cancellation.Cancel(); Tablet current = tablet; if (current != null) current.Cancel();
            if (work != null) await work;
            work = null; cancellation.Dispose(); cancellation = null;
            start.Enabled = monitors.Enabled = ink.Enabled = true; stop.Enabled = false; status.Text = "Остановлен";
            tray.Text = "Bamboo CTE-450 · Остановлен";
            Program.Log("SESSION packets=" + packets + "; max pressure=" + maximum + "/511");
            stopping = false; timer.Start();
        }
    }

    // End-to-end native test: only inject into this isolated test window.
    internal sealed class InkTestForm : Form {
        private Pen pen;
        private readonly System.Windows.Forms.Timer clock = new System.Windows.Forms.Timer();
        private int step, downs, ups, minimum = 1024, maximum;
        private bool rectangles;
        internal int Result = 1;
        internal InkTestForm() {
            Text = "Bamboo · Windows Ink test"; ClientSize = new Size(260, 100);
            StartPosition = FormStartPosition.CenterScreen; TopMost = true;
            Shown += delegate {
                try { pen = new Pen(); clock.Interval = 60; clock.Start(); }
                catch (Exception error) { Program.Log("INK TEST FAILED: " + error); Close(); }
            };
            clock.Tick += delegate {
                try {
                    int[] pressures = { 0, 64, 256, 511, 0 };
                    if (step < pressures.Length) {
                        int force = pressures[step++];
                        Report r = new Report { X = 7380, Y = 4612, InRange = true, Contact = force > 0, Pressure = force };
                        pen.Update(r, RectangleToScreen(ClientRectangle));
                    } else if (step++ > 6) {
                        Result = downs > 0 && ups > 0 && minimum < maximum && maximum == 1024 ? 0 : 1;
                        Program.Log("INK TEST " + (Result == 0 ? "PASS" : "FAILED") + ": down=" + downs + "; up=" + ups + "; pressure=" + minimum + ".." + maximum + "; device rects=" + rectangles);
                        Close();
                    }
                } catch (Exception error) { Program.Log("INK TEST FAILED: " + error); Close(); }
            };
            FormClosed += delegate { clock.Dispose(); if (pen != null) pen.Dispose(); };
        }
        protected override void WndProc(ref Message message) {
            if (message.Msg == 0x245 || message.Msg == 0x246 || message.Msg == 0x247) {
                Native.PenInfo data;
                if (Native.GetPointerPenInfo((uint)message.WParam.ToInt64() & 0xffff, out data)) {
                    if (message.Msg == 0x246) downs++;
                    if (message.Msg == 0x247) ups++;
                    if ((data.Mask & 1) != 0 && data.Pressure > 0) {
                        minimum = Math.Min(minimum, (int)data.Pressure); maximum = Math.Max(maximum, (int)data.Pressure);
                    }
                    Rectangle device, screen;
                    rectangles |= Native.GetPointerDeviceRects(data.Pointer.Source, out device, out screen);
                }
                message.Result = IntPtr.Zero; return;
            }
            base.WndProc(ref message);
        }
    }

    internal static class Program {
        internal static readonly string LogFile = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "BambooCTE450", "driver.log");
        internal static void Log(string text) {
            try { Directory.CreateDirectory(Path.GetDirectoryName(LogFile)); File.AppendAllText(LogFile, DateTime.Now.ToString("s") + " " + text + Environment.NewLine); } catch { }
        }
        [STAThread] internal static int Main(string[] args) {
            Native.SetProcessDpiAwarenessContext(new IntPtr(-4));
            if (args.Length != 0 && args[0] == "--self-test") return SelfTest();
            if (args.Length != 0 && args[0] == "--ink-test") {
                Application.EnableVisualStyles(); using (InkTestForm form = new InkTestForm()) { Application.Run(form); return form.Result; }
            }
            bool owner;
            using (Mutex singleton = new Mutex(true, "Local\\BambooCTE450Driver", out owner)) {
                if (!owner) {
                    if (args.Length != 0) { Log("Driver busy; stop the existing instance before probing"); return 2; }
                    MessageBox.Show("Драйвер уже запущен. Откройте его значок в трее.", "Bamboo CTE-450"); return 0;
                }
                if (args.Length != 0 && args[0] == "--probe") {
                    try { using (Tablet device = new Tablet()) { device.Start(); Log("PROBE: HID opened; mode read, tablet mode enabled, original mode will be restored"); } return 0; }
                    catch (Exception error) { Log("PROBE FAILED: " + error); return 1; }
                }
                Application.EnableVisualStyles(); Application.SetCompatibleTextRenderingDefault(false);
                Application.Run(new DriverForm()); return 0;
            }
        }
        private static int SelfTest() {
            try {
                Action<bool, string> check = delegate(bool value, string message) { if (!value) throw new Exception(message); };
                check(Marshal.SizeOf(typeof(Native.PointerInfo)) == 96, "POINTER_INFO ABI");
                check(Marshal.SizeOf(typeof(Native.PenInfo)) == 120, "POINTER_PEN_INFO ABI");
                check(Marshal.SizeOf(typeof(Native.TouchInfo)) == 144, "POINTER_TOUCH_INFO ABI");
                Report r;
                check(Report.Parse(new byte[] {2, 0x81, 0xa8, 0x39, 9, 0x24, 0xff, 1, 0}, out r), "pen report");
                check(r.X == 14760 && r.Y == 9225 && r.Pressure == 511 && r.Contact, "decode maximum");
                check(r.Map(new Rectangle(-1920, 100, 1920, 1080)) == new Point(-1, 1179), "negative monitor coordinates");
                check(Report.Parse(new byte[] {2, 0x80, 1, 0, 1, 0, 0xff, 1, 0}, out r) && r.Pressure == 0 && !r.Contact, "hover has no pressure");
                check(!Report.Parse(new byte[] {2, 0xc1, 1, 0, 1, 0, 0xff, 1, 0}, out r), "exclude tablet mouse");
                check(!Report.Parse(new byte[] {2, 0, 0, 0, 0, 0, 0, 8, 0}, out r), "exclude pad");
                check(!Report.Parse(new byte[8], out r), "truncated report");
                using (Pen pen = new Pen()) { } // Validate availability without moving the pointer.
                Log("SELF TEST PASS: ABI, decoding, hover, pressure, mapping, synthetic pen creation");
                return 0;
            } catch (Exception error) { Log("SELF TEST FAILED: " + error); return 1; }
        }
    }
}
