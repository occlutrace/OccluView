[CmdletBinding()]
param(
    [string]$SamplePath = "",
    [int]$Width = 640,
    [int]$Height = 480,
    [int]$ResizeWidth = 320,
    [int]$ResizeHeight = 180,
    [string]$PreviewClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function New-SmokeStl {
    $path = Join-Path $env:TEMP "occluview-preview-smoke.stl"
    $triangles = @(
        @(
            [single]0.0, [single]-0.6, [single]0.8,
            [single]-1.2, [single]-0.7, [single]-0.4,
            [single]1.4, [single]-0.9, [single]-0.6,
            [single]0.1, [single]1.1, [single]0.2
        ),
        @(
            [single]-0.8, [single]0.3, [single]0.9,
            [single]-1.2, [single]-0.7, [single]-0.4,
            [single]0.1, [single]1.1, [single]0.2,
            [single]-0.5, [single]0.0, [single]1.6
        ),
        @(
            [single]0.9, [single]0.2, [single]0.7,
            [single]0.1, [single]1.1, [single]0.2,
            [single]1.4, [single]-0.9, [single]-0.6,
            [single]-0.5, [single]0.0, [single]1.6
        ),
        @(
            [single]0.0, [single]-1.0, [single]-0.2,
            [single]1.4, [single]-0.9, [single]-0.6,
            [single]-1.2, [single]-0.7, [single]-0.4,
            [single]-0.3, [single]-0.2, [single]-1.0
        )
    )
    $bytes = New-Object byte[] (84 + (50 * $triangles.Count))
    $header = [Text.Encoding]::ASCII.GetBytes("occluview preview smoke")
    [Array]::Copy($header, 0, $bytes, 0, $header.Length)
    [BitConverter]::GetBytes([uint32]$triangles.Count).CopyTo($bytes, 80)

    $offset = 84
    foreach ($triangle in $triangles) {
        foreach ($value in $triangle) {
            [BitConverter]::GetBytes($value).CopyTo($bytes, $offset)
            $offset += 4
        }
        $offset += 2
    }
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Threading;

public static class OccluViewShellPreviewSmoke
{
    private const uint COINIT_APARTMENTTHREADED = 0x2;
    private const string PreviewChildClass = "OccluViewPreviewPane";
    private const int WM_RBUTTONDOWN = 0x0204;
    private const int WM_RBUTTONUP = 0x0205;
    private const int WM_MOUSEMOVE = 0x0200;
    private const int WM_MOUSEWHEEL = 0x020A;
    private const uint MK_RBUTTON = 0x0002;
    private const uint BI_RGB = 0;
    private const uint DIB_RGB_COLORS = 0;
    private const uint SRCCOPY = 0x00CC0020;
    private const uint WS_POPUP = 0x80000000;
    private const uint WS_VISIBLE = 0x10000000;
    private const int SW_SHOWNOACTIVATE = 4;
    private const uint WM_KEYDOWN = 0x0100;
    private const uint VK_F6 = 0x75;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT
    {
        public int left;
        public int top;
        public int right;
        public int bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAP
    {
        public int bmType;
        public int bmWidth;
        public int bmHeight;
        public int bmWidthBytes;
        public ushort bmPlanes;
        public ushort bmBitsPixel;
        public IntPtr bmBits;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAPINFOHEADER
    {
        public uint biSize;
        public int biWidth;
        public int biHeight;
        public ushort biPlanes;
        public ushort biBitCount;
        public uint biCompression;
        public uint biSizeImage;
        public int biXPelsPerMeter;
        public int biYPelsPerMeter;
        public uint biClrUsed;
        public uint biClrImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RGBQUAD
    {
        public byte rgbBlue;
        public byte rgbGreen;
        public byte rgbRed;
        public byte rgbReserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT
    {
        public int x;
        public int y;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MSG
    {
        public IntPtr hwnd;
        public uint message;
        public IntPtr wParam;
        public IntPtr lParam;
        public uint time;
        public POINT pt;
        public uint lPrivate;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAPINFO
    {
        public BITMAPINFOHEADER bmiHeader;
        public RGBQUAD bmiColors;
    }

    private sealed class FrameProbe
    {
        public int VisiblePixels { get; set; }
        public ulong Hash { get; set; }

        public string Summary()
        {
            return "visible=" + VisiblePixels + " hash=0x" + Hash.ToString("X16");
        }
    }

    [ComImport]
    [Guid("B7D14566-0509-4CCE-A71F-0A554233BD9B")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IInitializeWithFile
    {
        void Initialize([MarshalAs(UnmanagedType.LPWStr)] string pszFilePath, uint grfMode);
    }

    [ComImport]
    [Guid("B824B49D-22AC-4161-AC8A-9916E8FA3F7F")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IInitializeWithStream
    {
        void Initialize([MarshalAs(UnmanagedType.Interface)] object pstream, uint grfMode);
    }

    [ComImport]
    [Guid("7F73BE3F-FB79-493C-A6C7-7EE14E245841")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IInitializeWithItem
    {
        void Initialize(IShellItem psi, uint grfMode);
    }

    [ComImport]
    [Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IShellItem
    {
    }

    [ComImport]
    [Guid("8895B1C6-B41F-4C1C-A562-0D564250836F")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPreviewHandler
    {
        void SetWindow(IntPtr hwnd, ref RECT prc);
        void SetRect(ref RECT prc);
        void DoPreview();
        void Unload();
        void SetFocus();
        IntPtr QueryFocus();
        [PreserveSig]
        int TranslateAccelerator(ref MSG pmsg);
    }

    [DllImport("ole32.dll")]
    private static extern int CoInitializeEx(IntPtr pvReserved, uint dwCoInit);

    [DllImport("ole32.dll")]
    private static extern void CoUninitialize();

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode, PreserveSig = true)]
    private static extern int SHCreateStreamOnFileEx(
        string pszFile,
        uint grfMode,
        uint dwAttributes,
        bool fCreate,
        [MarshalAs(UnmanagedType.Interface)] object pstmTemplate,
        [MarshalAs(UnmanagedType.Interface)] out object ppstm);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, EntryPoint = "SHCreateItemFromParsingName", PreserveSig = false)]
    private static extern void SHCreateShellItemFromParsingName(
        [MarshalAs(UnmanagedType.LPWStr)] string path,
        IntPtr bindContext,
        [In] ref Guid riid,
        [MarshalAs(UnmanagedType.Interface)] out IShellItem item);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(
        uint dwExStyle,
        string lpClassName,
        string lpWindowName,
        uint dwStyle,
        int x,
        int y,
        int nWidth,
        int nHeight,
        IntPtr hWndParent,
        IntPtr hMenu,
        IntPtr hInstance,
        IntPtr lpParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool DestroyWindow(IntPtr hWnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr FindWindowExW(
        IntPtr hWndParent,
        IntPtr hWndChildAfter,
        string lpszClass,
        string lpszWindow);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetClassNameW(
        IntPtr hWnd,
        char[] lpClassName,
        int nMaxCount);

    [DllImport("user32.dll")]
    private static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UpdateWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SendMessageW(
        IntPtr hWnd,
        uint Msg,
        IntPtr wParam,
        IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern IntPtr GetDC(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateCompatibleDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int nWidth, int nHeight);

    [DllImport("gdi32.dll")]
    private static extern IntPtr SelectObject(IntPtr hdc, IntPtr h);

    [DllImport("gdi32.dll", SetLastError = true)]
    private static extern bool BitBlt(
        IntPtr hdc,
        int x,
        int y,
        int cx,
        int cy,
        IntPtr hdcSrc,
        int x1,
        int y1,
        uint rop);

    [DllImport("gdi32.dll")]
    private static extern int GetObject(IntPtr hgdiobj, int cbBuffer, out BITMAP lpvObject);

    [DllImport("gdi32.dll", SetLastError = true)]
    private static extern int GetDIBits(
        IntPtr hdc,
        IntPtr hbm,
        uint uStartScan,
        uint cScanLines,
        [Out] byte[] lpvBits,
        ref BITMAPINFO lpbi,
        uint uUsage);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteObject(IntPtr hObject);

    public static string Probe(
        string previewClsid,
        string path,
        int width,
        int height,
        int resizeWidth,
        int resizeHeight,
        bool useStream)
    {
        string result = null;
        Exception failure = null;

        var thread = new Thread(() =>
        {
            IntPtr parent = IntPtr.Zero;
            object instance = null;
            object stream = null;
            bool coInitialized = false;
            var coinit = CoInitializeEx(IntPtr.Zero, COINIT_APARTMENTTHREADED);
            if (coinit < 0)
            {
                Marshal.ThrowExceptionForHR(coinit);
            }
            coInitialized = true;

            try
            {
                parent = CreateWindowExW(0, "Static", "OccluViewPreviewSmoke", WS_POPUP | WS_VISIBLE, 0, 0, width, height, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (parent == IntPtr.Zero)
                {
                    throw new InvalidOperationException("CreateWindowExW failed for preview smoke host parent window.");
                }
                ShowWindow(parent, SW_SHOWNOACTIVATE);
                UpdateWindow(parent);

                var type = Type.GetTypeFromCLSID(new Guid(previewClsid), true);
                instance = Activator.CreateInstance(type);

                var preview = (IPreviewHandler)instance;

                var initialRect = new RECT { left = 0, top = 0, right = width, bottom = height };
                if (useStream)
                {
                    int streamHr = SHCreateStreamOnFileEx(path, 0x00000020, 0, false, null, out stream);
                    if (streamHr < 0 || stream == null)
                    {
                        Marshal.ThrowExceptionForHR(streamHr);
                    }
                    ((IInitializeWithStream)instance).Initialize(stream, 0);
                }
                else
                {
                    ((IInitializeWithFile)instance).Initialize(path, 0);
                }
                preview.SetWindow(parent, ref initialRect);
                preview.DoPreview();

                var child = FindWindowExW(parent, IntPtr.Zero, PreviewChildClass, null);
                if (child == IntPtr.Zero)
                {
                    throw new InvalidOperationException("Preview handler created no child preview window.");
                }

                if (!UpdateWindow(child))
                {
                    throw new InvalidOperationException("UpdateWindow failed for initial preview child.");
                }

                EnsurePreviewChild(child, width, height, "initial preview host size");

                preview.SetFocus();
                var focused = preview.QueryFocus();
                if (focused == IntPtr.Zero)
                {
                    throw new InvalidOperationException("Preview handler QueryFocus returned a null HWND after SetFocus.");
                }
                if (focused != child)
                {
                    throw new InvalidOperationException("Preview handler QueryFocus returned the wrong HWND after SetFocus. Expected " + child + ", got " + focused + ".");
                }

                var accelerator = new MSG
                {
                    hwnd = child,
                    message = WM_KEYDOWN,
                    wParam = new IntPtr(VK_F6),
                    lParam = IntPtr.Zero,
                    time = 0,
                    pt = new POINT { x = 0, y = 0 },
                    lPrivate = 0
                };
                int translateResult = preview.TranslateAccelerator(ref accelerator);
                if (translateResult != 1)
                {
                    throw new InvalidOperationException("Preview handler TranslateAccelerator returned " + translateResult + " instead of S_FALSE.");
                }

                var resizedRect = new RECT { left = 0, top = 0, right = resizeWidth, bottom = resizeHeight };
                preview.SetRect(ref resizedRect);
                if (!UpdateWindow(child))
                {
                    throw new InvalidOperationException("UpdateWindow failed for resized preview child.");
                }

                EnsurePreviewChild(child, resizeWidth, resizeHeight, "resized preview host size");

                var initialFrame = CaptureFrame(child);
                EnsureFrameVisible(initialFrame, "initial resized preview frame");
                var orbitFrame = OrbitPreview(child, resizeWidth, resizeHeight);
                if (!FramesDiffer(initialFrame, orbitFrame))
                {
                    throw new InvalidOperationException("Preview orbit drag did not change the rendered frame. Initial=" + initialFrame.Summary() + " orbit=" + orbitFrame.Summary());
                }

                var zoomFrame = ZoomPreview(child);
                if (!FramesDiffer(orbitFrame, zoomFrame))
                {
                    throw new InvalidOperationException("Preview zoom did not change the rendered frame. Orbit=" + orbitFrame.Summary() + " zoom=" + zoomFrame.Summary());
                }

                preview.Unload();

                if (IsWindow(child))
                {
                    throw new InvalidOperationException("Preview handler left the child preview window alive after Unload.");
                }

                result = (useStream ? "stream " : "file ")
                    + width + "x" + height + " -> " + resizeWidth + "x" + resizeHeight
                    + " child " + PreviewChildClass
                    + " initial[" + initialFrame.Summary() + "]"
                    + " orbit[" + orbitFrame.Summary() + "]"
                    + " zoom[" + zoomFrame.Summary() + "]";
            }
            catch (Exception ex)
            {
                failure = ex;
            }
            finally
            {
                if (stream != null && Marshal.IsComObject(stream))
                {
                    Marshal.FinalReleaseComObject(stream);
                }
                if (instance != null && Marshal.IsComObject(instance))
                {
                    Marshal.FinalReleaseComObject(instance);
                }
                if (parent != IntPtr.Zero && IsWindow(parent))
                {
                    DestroyWindow(parent);
                }
                if (coInitialized)
                {
                    CoUninitialize();
                }
            }
        });

        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        thread.Join();

        if (failure != null)
        {
            throw new InvalidOperationException("Preview smoke failed: " + failure, failure);
        }

        return result ?? throw new InvalidOperationException("Preview smoke produced no result.");
    }

    public static string ProbeFromItem(
        string previewClsid,
        string path,
        int width,
        int height,
        int resizeWidth,
        int resizeHeight)
    {
        string result = null;
        Exception failure = null;

        var thread = new Thread(() =>
        {
            IntPtr parent = IntPtr.Zero;
            object instance = null;
            IShellItem item = null;
            bool coInitialized = false;
            var coinit = CoInitializeEx(IntPtr.Zero, COINIT_APARTMENTTHREADED);
            if (coinit < 0)
            {
                Marshal.ThrowExceptionForHR(coinit);
            }
            coInitialized = true;

            try
            {
                parent = CreateWindowExW(0, "Static", "OccluViewPreviewSmoke", WS_POPUP | WS_VISIBLE, 0, 0, width, height, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (parent == IntPtr.Zero)
                {
                    throw new InvalidOperationException("CreateWindowExW failed for preview smoke host parent window.");
                }
                ShowWindow(parent, SW_SHOWNOACTIVATE);
                UpdateWindow(parent);

                var type = Type.GetTypeFromCLSID(new Guid(previewClsid), true);
                instance = Activator.CreateInstance(type);

                var preview = (IPreviewHandler)instance;
                var initialRect = new RECT { left = 0, top = 0, right = width, bottom = height };
                Guid shellItemIid = typeof(IShellItem).GUID;
                SHCreateShellItemFromParsingName(path, IntPtr.Zero, ref shellItemIid, out item);
                if (item == null)
                {
                    throw new InvalidOperationException("SHCreateItemFromParsingName returned a null shell item.");
                }
                ((IInitializeWithItem)instance).Initialize(item, 0);
                preview.SetWindow(parent, ref initialRect);
                preview.DoPreview();

                var child = FindWindowExW(parent, IntPtr.Zero, PreviewChildClass, null);
                if (child == IntPtr.Zero)
                {
                    throw new InvalidOperationException("Preview handler created no child preview window.");
                }

                if (!UpdateWindow(child))
                {
                    throw new InvalidOperationException("UpdateWindow failed for initial preview child.");
                }

                EnsurePreviewChild(child, width, height, "initial preview host size");

                preview.SetFocus();
                var focused = preview.QueryFocus();
                if (focused == IntPtr.Zero)
                {
                    throw new InvalidOperationException("Preview handler QueryFocus returned a null HWND after SetFocus.");
                }
                if (focused != child)
                {
                    throw new InvalidOperationException("Preview handler QueryFocus returned the wrong HWND after SetFocus. Expected " + child + ", got " + focused + ".");
                }

                var accelerator = new MSG
                {
                    hwnd = child,
                    message = WM_KEYDOWN,
                    wParam = new IntPtr(VK_F6),
                    lParam = IntPtr.Zero,
                    time = 0,
                    pt = new POINT { x = 0, y = 0 },
                    lPrivate = 0
                };
                int translateResult = preview.TranslateAccelerator(ref accelerator);
                if (translateResult != 1)
                {
                    throw new InvalidOperationException("Preview handler TranslateAccelerator returned " + translateResult + " instead of S_FALSE.");
                }

                var resizedRect = new RECT { left = 0, top = 0, right = resizeWidth, bottom = resizeHeight };
                preview.SetRect(ref resizedRect);
                if (!UpdateWindow(child))
                {
                    throw new InvalidOperationException("UpdateWindow failed for resized preview child.");
                }

                EnsurePreviewChild(child, resizeWidth, resizeHeight, "resized preview host size");

                var initialFrame = CaptureFrame(child);
                EnsureFrameVisible(initialFrame, "initial resized preview frame");
                var orbitFrame = OrbitPreview(child, resizeWidth, resizeHeight);
                if (!FramesDiffer(initialFrame, orbitFrame))
                {
                    throw new InvalidOperationException("Preview orbit drag did not change the rendered frame. Initial=" + initialFrame.Summary() + " orbit=" + orbitFrame.Summary());
                }

                var zoomFrame = ZoomPreview(child);
                if (!FramesDiffer(orbitFrame, zoomFrame))
                {
                    throw new InvalidOperationException("Preview zoom did not change the rendered frame. Orbit=" + orbitFrame.Summary() + " zoom=" + zoomFrame.Summary());
                }

                preview.Unload();

                if (IsWindow(child))
                {
                    throw new InvalidOperationException("Preview handler left the child preview window alive after Unload.");
                }

                result = "item "
                    + width + "x" + height + " -> " + resizeWidth + "x" + resizeHeight
                    + " child " + PreviewChildClass
                    + " initial[" + initialFrame.Summary() + "]"
                    + " orbit[" + orbitFrame.Summary() + "]"
                    + " zoom[" + zoomFrame.Summary() + "]";
            }
            catch (Exception ex)
            {
                failure = ex;
            }
            finally
            {
                if (item != null && Marshal.IsComObject(item))
                {
                    Marshal.FinalReleaseComObject(item);
                }
                if (instance != null && Marshal.IsComObject(instance))
                {
                    Marshal.FinalReleaseComObject(instance);
                }
                if (parent != IntPtr.Zero && IsWindow(parent))
                {
                    DestroyWindow(parent);
                }
                if (coInitialized)
                {
                    CoUninitialize();
                }
            }
        });

        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        thread.Join();

        if (failure != null)
        {
            throw new InvalidOperationException("Preview smoke failed: " + failure, failure);
        }

        return result ?? throw new InvalidOperationException("Preview smoke produced no result.");
    }

    private static void EnsurePreviewChild(IntPtr hwnd, int expectedWidth, int expectedHeight, string label)
    {
        EnsureClientRect(hwnd, expectedWidth, expectedHeight, label);

        char[] classBuffer = new char[128];
        int copied = GetClassNameW(hwnd, classBuffer, classBuffer.Length);
        if (copied <= 0)
        {
            throw new InvalidOperationException(label + " GetClassNameW failed.");
        }

        string className = new string(classBuffer, 0, copied);
        if (!string.Equals(className, PreviewChildClass, StringComparison.Ordinal))
        {
            throw new InvalidOperationException(label + " class mismatch. Expected " + PreviewChildClass + ", got " + className + ".");
        }
    }

    private static void EnsureClientRect(IntPtr hwnd, int expectedWidth, int expectedHeight, string label)
    {
        if (!GetClientRect(hwnd, out RECT rect))
        {
            throw new InvalidOperationException(label + " GetClientRect failed.");
        }

        int actualWidth = rect.right - rect.left;
        int actualHeight = rect.bottom - rect.top;
        if (actualWidth != expectedWidth || actualHeight != expectedHeight)
        {
            throw new InvalidOperationException(label + " mismatch. Expected " + expectedWidth + "x" + expectedHeight + ", got " + actualWidth + "x" + actualHeight + ".");
        }
    }

    private static FrameProbe OrbitPreview(IntPtr hwnd, int width, int height)
    {
        int centerX = Math.Max(8, width / 2);
        int centerY = Math.Max(8, height / 2);
        int dragX = Math.Min(width - 8, centerX + Math.Max(40, width / 4));
        int dragY = Math.Min(height - 8, centerY + Math.Max(28, height / 5));

        SendMessageW(hwnd, WM_RBUTTONDOWN, new IntPtr(MK_RBUTTON), MakeLParam(centerX, centerY));
        SendMessageW(hwnd, WM_MOUSEMOVE, new IntPtr(MK_RBUTTON), MakeLParam((centerX + dragX) / 2, (centerY + dragY) / 2));
        SendMessageW(hwnd, WM_MOUSEMOVE, new IntPtr(MK_RBUTTON), MakeLParam(dragX, dragY));
        SendMessageW(hwnd, WM_RBUTTONUP, IntPtr.Zero, MakeLParam(dragX, dragY));
        if (!UpdateWindow(hwnd))
        {
            throw new InvalidOperationException("UpdateWindow failed after preview orbit drag.");
        }
        return CaptureFrame(hwnd);
    }

    private static FrameProbe ZoomPreview(IntPtr hwnd)
    {
        IntPtr wheelWParam = new IntPtr(240 << 16);
        SendMessageW(hwnd, WM_MOUSEWHEEL, wheelWParam, IntPtr.Zero);
        if (!UpdateWindow(hwnd))
        {
            throw new InvalidOperationException("UpdateWindow failed after preview zoom.");
        }
        return CaptureFrame(hwnd);
    }

    private static void EnsureFrameVisible(FrameProbe frame, string label)
    {
        if (frame.VisiblePixels < 64)
        {
            throw new InvalidOperationException(label + " did not contain enough visible geometry. " + frame.Summary());
        }
    }

    private static bool FramesDiffer(FrameProbe before, FrameProbe after)
    {
        if (before.Hash != after.Hash)
        {
            return true;
        }

        int visibleDelta = Math.Abs(before.VisiblePixels - after.VisiblePixels);
        int threshold = Math.Max(32, Math.Min(before.VisiblePixels, after.VisiblePixels) / 20);
        return visibleDelta >= threshold;
    }

    private static IntPtr MakeLParam(int x, int y)
    {
        return new IntPtr((y << 16) | (x & 0xFFFF));
    }

    private static FrameProbe CaptureFrame(IntPtr hwnd)
    {
        if (!GetClientRect(hwnd, out RECT rect))
        {
            throw new InvalidOperationException("GetClientRect failed while capturing preview frame.");
        }
        int width = rect.right - rect.left;
        int height = rect.bottom - rect.top;
        if (width <= 0 || height <= 0)
        {
            throw new InvalidOperationException("Preview child had invalid capture size " + width + "x" + height + ".");
        }

        IntPtr windowDc = GetDC(hwnd);
        if (windowDc == IntPtr.Zero)
        {
            throw new InvalidOperationException("GetDC returned a null HDC for preview child.");
        }

        IntPtr memoryDc = IntPtr.Zero;
        IntPtr bitmap = IntPtr.Zero;
        IntPtr previous = IntPtr.Zero;
        try
        {
            memoryDc = CreateCompatibleDC(windowDc);
            if (memoryDc == IntPtr.Zero)
            {
                throw new InvalidOperationException("CreateCompatibleDC failed for preview capture.");
            }

            bitmap = CreateCompatibleBitmap(windowDc, width, height);
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("CreateCompatibleBitmap failed for preview capture.");
            }

            previous = SelectObject(memoryDc, bitmap);
            if (previous == IntPtr.Zero)
            {
                throw new InvalidOperationException("SelectObject failed for preview capture bitmap.");
            }

            if (!BitBlt(memoryDc, 0, 0, width, height, windowDc, 0, 0, SRCCOPY))
            {
                throw new InvalidOperationException("BitBlt failed while capturing preview child.");
            }

            BITMAP info;
            int copied = GetObject(bitmap, Marshal.SizeOf<BITMAP>(), out info);
            if (copied == 0 || info.bmWidth <= 0 || info.bmHeight <= 0)
            {
                throw new InvalidOperationException("Captured preview bitmap was invalid.");
            }

            byte[] pixels = ReadPixels(memoryDc, bitmap, info.bmWidth, info.bmHeight);
            return Analyze(pixels, info.bmWidth, info.bmHeight);
        }
        finally
        {
            if (previous != IntPtr.Zero && memoryDc != IntPtr.Zero)
            {
                SelectObject(memoryDc, previous);
            }
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (memoryDc != IntPtr.Zero)
            {
                DeleteDC(memoryDc);
            }
            ReleaseDC(hwnd, windowDc);
        }
    }

    private static byte[] ReadPixels(IntPtr hdc, IntPtr bitmap, int width, int height)
    {
        var bmi = new BITMAPINFO
        {
            bmiHeader = new BITMAPINFOHEADER
            {
                biSize = (uint)Marshal.SizeOf<BITMAPINFOHEADER>(),
                biWidth = width,
                biHeight = -height,
                biPlanes = 1,
                biBitCount = 32,
                biCompression = BI_RGB,
                biSizeImage = (uint)(width * height * 4)
            }
        };

        byte[] pixels = new byte[width * height * 4];
        int scanLines = GetDIBits(hdc, bitmap, 0, (uint)height, pixels, ref bmi, DIB_RGB_COLORS);
        if (scanLines == 0)
        {
            throw new InvalidOperationException("GetDIBits failed for preview capture.");
        }
        return pixels;
    }

    private static FrameProbe Analyze(byte[] pixels, int width, int height)
    {
        byte[] background = EstimateBackground(pixels, width, height);
        const int diffThreshold = 44;

        int visiblePixels = 0;
        for (int y = 0; y < height; y++)
        {
            for (int x = 0; x < width; x++)
            {
                int offset = ((y * width) + x) * 4;
                int diff =
                    Math.Abs(pixels[offset] - background[0]) +
                    Math.Abs(pixels[offset + 1] - background[1]) +
                    Math.Abs(pixels[offset + 2] - background[2]) +
                    Math.Abs(pixels[offset + 3] - background[3]);
                if (diff > diffThreshold)
                {
                    visiblePixels++;
                }
            }
        }

        return new FrameProbe
        {
            VisiblePixels = visiblePixels,
            Hash = ComputeDifferenceHash(pixels, width, height, background)
        };
    }

    private static byte[] EstimateBackground(byte[] pixels, int width, int height)
    {
        long blue = 0;
        long green = 0;
        long red = 0;
        long alpha = 0;
        int count = 0;

        int[,] corners = new int[,]
        {
            { 0, 0 },
            { width - 1, 0 },
            { 0, height - 1 },
            { width - 1, height - 1 }
        };

        for (int corner = 0; corner < corners.GetLength(0); corner++)
        {
            int baseX = corners[corner, 0];
            int baseY = corners[corner, 1];
            for (int dy = 0; dy < 2; dy++)
            {
                for (int dx = 0; dx < 2; dx++)
                {
                    int x = Math.Max(0, Math.Min(width - 1, baseX + (baseX == 0 ? dx : -dx)));
                    int y = Math.Max(0, Math.Min(height - 1, baseY + (baseY == 0 ? dy : -dy)));
                    int offset = ((y * width) + x) * 4;
                    blue += pixels[offset];
                    green += pixels[offset + 1];
                    red += pixels[offset + 2];
                    alpha += pixels[offset + 3];
                    count++;
                }
            }
        }

        return new[]
        {
            (byte)(blue / count),
            (byte)(green / count),
            (byte)(red / count),
            (byte)(alpha / count)
        };
    }

    private static ulong ComputeDifferenceHash(byte[] pixels, int width, int height, byte[] background)
    {
        const int sampleWidth = 9;
        const int sampleHeight = 8;
        byte[] samples = new byte[sampleWidth * sampleHeight];

        for (int y = 0; y < sampleHeight; y++)
        {
            int sourceY = sampleHeight == 1 ? 0 : y * (height - 1) / (sampleHeight - 1);
            for (int x = 0; x < sampleWidth; x++)
            {
                int sourceX = sampleWidth == 1 ? 0 : x * (width - 1) / (sampleWidth - 1);
                int offset = ((sourceY * width) + sourceX) * 4;
                int diff =
                    Math.Abs(pixels[offset] - background[0]) +
                    Math.Abs(pixels[offset + 1] - background[1]) +
                    Math.Abs(pixels[offset + 2] - background[2]) +
                    Math.Abs(pixels[offset + 3] - background[3]);
                samples[(y * sampleWidth) + x] = (byte)Math.Min(255, diff / 2);
            }
        }

        ulong hash = 0;
        int bitIndex = 0;
        for (int y = 0; y < sampleHeight; y++)
        {
            int row = y * sampleWidth;
            for (int x = 0; x < sampleWidth - 1; x++)
            {
                if (samples[row + x] <= samples[row + x + 1])
                {
                    hash |= 1UL << bitIndex;
                }
                bitIndex++;
            }
        }
        return hash;
    }
}
"@

if ([string]::IsNullOrWhiteSpace($SamplePath)) {
    $SamplePath = New-SmokeStl
} else {
    $SamplePath = (Resolve-Path $SamplePath).Path
}

if ($Width -lt 64 -or $Height -lt 64 -or $ResizeWidth -lt 64 -or $ResizeHeight -lt 64) {
    throw "Preview smoke dimensions must be at least 64 px."
}

$fileResult = [OccluViewShellPreviewSmoke]::Probe(
    $PreviewClsid,
    $SamplePath,
    $Width,
    $Height,
    $ResizeWidth,
    $ResizeHeight,
    $false
)

$streamResult = [OccluViewShellPreviewSmoke]::Probe(
    $PreviewClsid,
    $SamplePath,
    $Width,
    $Height,
    $ResizeWidth,
    $ResizeHeight,
    $true
)

$itemResult = [OccluViewShellPreviewSmoke]::ProbeFromItem(
    $PreviewClsid,
    $SamplePath,
    $Width,
    $Height,
    $ResizeWidth,
    $ResizeHeight
)

Write-Host "Preview smoke: file[$fileResult] stream[$streamResult] item[$itemResult]"
