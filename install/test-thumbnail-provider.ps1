[CmdletBinding()]
param(
    [string]$SamplePath = "",
    [int[]]$Sizes = @(16, 32, 96, 256, 1024)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$thumbnailProviderClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3045}"

Add-Type -AssemblyName System.IO.Compression -ErrorAction Stop
Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop

function New-SmokeStl {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-smoke.stl"
    $vertices = @(
        @(-1.0, -1.0, -1.0),
        @(1.0, -1.0, -1.0),
        @(1.0, 1.0, -1.0),
        @(-1.0, 1.0, -1.0),
        @(-1.0, -1.0, 1.0),
        @(1.0, -1.0, 1.0),
        @(1.0, 1.0, 1.0),
        @(-1.0, 1.0, 1.0)
    )
    $triangles = @(
        @(@(0.0, 0.0, -1.0), @(0, 2, 1)),
        @(@(0.0, 0.0, -1.0), @(0, 3, 2)),
        @(@(0.0, 0.0, 1.0), @(4, 5, 6)),
        @(@(0.0, 0.0, 1.0), @(4, 6, 7)),
        @(@(0.0, -1.0, 0.0), @(0, 1, 5)),
        @(@(0.0, -1.0, 0.0), @(0, 5, 4)),
        @(@(1.0, 0.0, 0.0), @(1, 2, 6)),
        @(@(1.0, 0.0, 0.0), @(1, 6, 5)),
        @(@(0.0, 1.0, 0.0), @(2, 3, 7)),
        @(@(0.0, 1.0, 0.0), @(2, 7, 6)),
        @(@(-1.0, 0.0, 0.0), @(3, 0, 4)),
        @(@(-1.0, 0.0, 0.0), @(3, 4, 7))
    )

    $header = [Text.Encoding]::ASCII.GetBytes("OccluView cube STL")
    $array = New-Object byte[] 84
    [Array]::Copy($header, 0, $array, 0, $header.Length)
    [BitConverter]::GetBytes([uint32]$triangles.Count).CopyTo($array, 80)
    $bytes = New-Object System.Collections.Generic.List[byte]
    $bytes.AddRange($array)

    foreach ($triangle in $triangles) {
        $normal = $triangle[0]
        $indices = $triangle[1]
        foreach ($value in $normal) {
            $bytes.AddRange([BitConverter]::GetBytes([single]$value))
        }
        foreach ($vertexIndex in $indices) {
            foreach ($value in $vertices[$vertexIndex]) {
                $bytes.AddRange([BitConverter]::GetBytes([single]$value))
            }
        }
        $bytes.AddRange([byte[]](0, 0))
    }

    [IO.File]::WriteAllBytes($path, $bytes.ToArray())
    return $path
}

function New-TruncatedStl {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-truncated.stl"
    $bytes = [IO.File]::ReadAllBytes((New-SmokeStl -Directory $Directory))
    $truncatedLength = 128
    if ($bytes.Length -lt $truncatedLength) {
        $truncatedLength = $bytes.Length
    }
    $truncated = New-Object byte[] $truncatedLength
    [Array]::Copy($bytes, 0, $truncated, 0, $truncatedLength)
    [IO.File]::WriteAllBytes($path, $truncated)
    return $path
}

function New-GarbageStl {
    param([Parameter(Mandatory = $true)][string]$Directory)

    # Deterministic non-STL bytes under an .stl name: a recognized extension
    # with unparseable content, distinct from the truncated fixture.
    $path = Join-Path $Directory "occluview-thumbnail-garbage.stl"
    $bytes = New-Object byte[] 256
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        $bytes[$i] = [byte]((($i * 73) + 41) % 251)
    }
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

function New-SmokePly {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-smoke.ply"
    $contents = @'
ply
format ascii 1.0
element vertex 8
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 12
property list uchar int vertex_indices
end_header
-1 -1 -1 220 180 105
1 -1 -1 215 170 96
1 1 -1 205 160 88
-1 1 -1 210 166 92
-1 -1 1 235 196 122
1 -1 1 226 184 112
1 1 1 218 176 104
-1 1 1 230 190 118
3 0 2 1
3 0 3 2
3 4 5 6
3 4 6 7
3 0 1 5
3 0 5 4
3 1 2 6
3 1 6 5
3 2 3 7
3 2 7 6
3 3 0 4
3 3 4 7
'@
    [IO.File]::WriteAllText($path, $contents, [Text.UTF8Encoding]::new($false))
    return $path
}

function New-SmokeObj {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-smoke.obj"
    $contents = @'
# OccluView thumbnail smoke OBJ
v -1.2 -0.8 -0.2 255 216 160
v 1.0 -0.7 -0.4 232 190 128
v 1.2 0.8 -0.1 210 168 104
v -0.8 1.0 0.2 244 202 144
v -0.2 -0.1 1.4 255 230 180
f 1 2 5
f 2 3 5
f 3 4 5
f 4 1 5
f 1 4 3
f 1 3 2
'@
    [IO.File]::WriteAllText($path, $contents, [Text.UTF8Encoding]::new($false))
    return $path
}

function New-NoisyObj {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [string]$Name = "occluview-thumbnail-noisy.obj",
        [int]$MinimumBytes = 819200
    )

    $path = Join-Path $Directory $Name
    $builder = [Text.StringBuilder]::new($MinimumBytes + 4096)
    [void]$builder.AppendLine("mtllib missing-materials.mtl")
    [void]$builder.AppendLine("usemtl scan")
    $invariant = [Globalization.CultureInfo]::InvariantCulture
    $baseIndex = 1
    $tile = 0
    while ($builder.Length -lt $MinimumBytes) {
        [void]$builder.AppendLine(("f {0}/{0} {1}/{1} {2}/{2}" -f $baseIndex, ($baseIndex + 1), ($baseIndex + 2)))
        $x = ($tile % 192) * 0.25
        $y = [Math]::Floor($tile / 192) * 0.25
        $z = ($tile % 17) * 0.02
        $xText = ([double]$x).ToString("R", $invariant)
        $yText = ([double]$y).ToString("R", $invariant)
        $zText = ([double]$z).ToString("R", $invariant)
        $xInsetText = ([double]($x + 0.18)).ToString("R", $invariant)
        $yInsetText = ([double]($y + 0.18)).ToString("R", $invariant)
        [void]$builder.AppendLine(("vt 0 0"))
        [void]$builder.AppendLine("v $xText $yText $zText 220 180 105")
        [void]$builder.AppendLine(("vt 0.18 0"))
        [void]$builder.AppendLine("v $xInsetText $yText $zText 215 170 96")
        [void]$builder.AppendLine(("vt 0 0.18"))
        [void]$builder.AppendLine("v $xText $yInsetText $zText 235 196 122")
        $baseIndex += 3
        $tile += 1
    }

    $payload = [Text.Encoding]::UTF8.GetBytes($builder.ToString())
    $prefixBytes = New-Object System.Collections.Generic.List[byte]
    $prefixBytes.AddRange([byte[]](0xEF, 0xBB, 0xBF))
    $prefixBytes.AddRange([Text.Encoding]::ASCII.GetBytes("# scanner metadata "))
    $prefixBytes.AddRange([byte[]](0xFF, 0xFE, 0x0A))
    $prefix = $prefixBytes.ToArray()
    $bytes = New-Object byte[] ($prefix.Length + $payload.Length)
    [Array]::Copy($prefix, 0, $bytes, 0, $prefix.Length)
    [Array]::Copy($payload, 0, $bytes, $prefix.Length, $payload.Length)
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

function New-SmokeHps {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-smoke.hps"
    $contents = @'
<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
</HPS>
'@
    [IO.File]::WriteAllText($path, $contents.TrimStart(), [Text.UTF8Encoding]::new($false))
    return $path
}

function New-SmokeLegacyHps {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $path = Join-Path $Directory "occluview-thumbnail-smoke.dcm"
    $hps = @'
<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
</HPS>
'@

    if (Test-Path $path) {
        Remove-Item $path -Force
    }
    $archive = [IO.Compression.ZipFile]::Open($path, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $entry = $archive.CreateEntry("scan/geometry.hps", [IO.Compression.CompressionLevel]::NoCompression)
        $stream = $entry.Open()
        try {
            $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
            try {
                $writer.Write($hps.TrimStart())
            } finally {
                $writer.Dispose()
            }
        } finally {
            $stream.Dispose()
        }
    } finally {
        $archive.Dispose()
    }
    return $path
}

Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;

public static class OccluViewShellThumbnailSmoke
{
    [StructLayout(LayoutKind.Sequential)]
    public struct SIZE
    {
        public int cx;
        public int cy;
    }

    [Flags]
    public enum SIIGBF : uint
    {
        RESIZETOFIT = 0x00,
        BIGGERSIZEOK = 0x01,
        THUMBNAILONLY = 0x08
    }

    public enum WTS_ALPHATYPE
    {
        WTSAT_UNKNOWN = 0,
        WTSAT_RGB = 1,
        WTSAT_ARGB = 2
    }

    [Flags]
    public enum WTS_FLAGS : uint
    {
        WTS_EXTRACT = 0x00,
        WTS_FORCEEXTRACTION = 0x04
    }

    [Flags]
    public enum WTS_CACHEFLAGS : uint
    {
        WTS_DEFAULT = 0x00,
        WTS_LOWQUALITY = 0x01,
        WTS_CACHED = 0x02
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
    private struct BITMAPINFO
    {
        public BITMAPINFOHEADER bmiHeader;
        public RGBQUAD bmiColors;
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b")]
    public interface IShellItemImageFactory
    {
        void GetImage(SIZE size, SIIGBF flags, out IntPtr phbm);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("b7d14566-0509-4cce-a71f-0a554233bd9b")]
    public interface IInitializeWithFile
    {
        void Initialize([MarshalAs(UnmanagedType.LPWStr)] string pszFilePath, uint grfMode);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("b824b49d-22ac-4161-ac8a-9916e8fa3f7f")]
    public interface IInitializeWithStream
    {
        void Initialize(IStream pstream, uint grfMode);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("7F73BE3F-FB79-493C-A6C7-7EE14E245841")]
    public interface IInitializeWithItem
    {
        void Initialize(IShellItem psi, uint grfMode);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE")]
    public interface IShellItem
    {
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("091162a4-bc96-411f-aae8-c5122cd03363")]
    public interface ISharedBitmap
    {
        void GetSharedBitmap(out IntPtr phbm);
        void GetSize(out SIZE size);
        void GetFormat(out WTS_ALPHATYPE alphaType);
        void InitializeBitmap(IntPtr bitmap, WTS_ALPHATYPE alphaType);
        void Detach(out IntPtr phbm);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("f676c15d-596a-4ce2-8234-33996f445db1")]
    public interface IThumbnailCache
    {
        void GetThumbnail(
            IShellItem shellItem,
            uint requestedSize,
            WTS_FLAGS flags,
            out ISharedBitmap thumbnail,
            out WTS_CACHEFLAGS cacheFlags,
            IntPtr thumbnailId);
    }

    [ComImport]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [Guid("e357fccd-a995-4576-b01f-234630154e96")]
    public interface IThumbnailProvider
    {
        void GetThumbnail(uint cx, out IntPtr phbmp, out WTS_ALPHATYPE pdwAlpha);
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    private static extern void SHCreateItemFromParsingName(
        [MarshalAs(UnmanagedType.LPWStr)] string path,
        IntPtr bindContext,
        [In] ref Guid riid,
        [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, EntryPoint = "SHCreateItemFromParsingName", PreserveSig = false)]
    private static extern void SHCreateShellItemFromParsingName(
        [MarshalAs(UnmanagedType.LPWStr)] string path,
        IntPtr bindContext,
        [In] ref Guid riid,
        [MarshalAs(UnmanagedType.Interface)] out IShellItem item);

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode, PreserveSig = true)]
    private static extern int SHCreateStreamOnFileEx(
        string pszFile,
        uint grfMode,
        uint dwAttributes,
        bool fCreate,
        [MarshalAs(UnmanagedType.Interface)] object pstmTemplate,
        out IStream ppstm);

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

    [DllImport("user32.dll")]
    private static extern IntPtr GetDC(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

    private const uint BI_RGB = 0;
    private const uint DIB_RGB_COLORS = 0;

    public sealed class ProbeResult
    {
        public string Route { get; set; }
        public int Width { get; set; }
        public int Height { get; set; }
        public int BitsPerPixel { get; set; }
        public int VisiblePixels { get; set; }
        public int ContentWidth { get; set; }
        public int ContentHeight { get; set; }
        public ulong Hash { get; set; }
        public long ElapsedMs { get; set; }

        public string Summary()
        {
            return Route + " " + Width + "x" + Height + " " + BitsPerPixel + "bpp visible=" + VisiblePixels +
                " bounds=" + ContentWidth + "x" + ContentHeight + " hash=0x" + Hash.ToString("X16") +
                " elapsed=" + ElapsedMs + "ms";
        }
    }

    public static ProbeResult ProbeShell(string path, int size)
    {
        Guid iid = typeof(IShellItemImageFactory).GUID;
        IShellItemImageFactory factory;
        SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out factory);

        IntPtr bitmap = IntPtr.Zero;
        try
        {
            var sw = Stopwatch.StartNew();
            factory.GetImage(
                new SIZE { cx = size, cy = size },
                SIIGBF.THUMBNAILONLY | SIIGBF.BIGGERSIZEOK,
                out bitmap);
            sw.Stop();
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Shell returned a null thumbnail bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "shell", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (factory != null)
            {
                Marshal.FinalReleaseComObject(factory);
            }
        }
    }

    public static ProbeResult ProbeShellForced(string path, int size)
    {
        const string localThumbnailCacheClsid = "50EF4544-AC9F-4A8E-B21B-8A26180DB13F";
        object cacheObject = null;
        IShellItem item = null;
        ISharedBitmap sharedBitmap = null;
        try
        {
            Guid iid = typeof(IShellItem).GUID;
            SHCreateShellItemFromParsingName(path, IntPtr.Zero, ref iid, out item);
            if (item == null)
            {
                throw new InvalidOperationException("SHCreateItemFromParsingName returned a null shell item.");
            }

            Type cacheType = Type.GetTypeFromCLSID(new Guid(localThumbnailCacheClsid), true);
            if (cacheType == null)
            {
                throw new InvalidOperationException("Local thumbnail cache COM type was not found.");
            }
            cacheObject = Activator.CreateInstance(cacheType);
            if (cacheObject == null)
            {
                throw new InvalidOperationException("Local thumbnail cache COM instance was not created.");
            }

            WTS_CACHEFLAGS cacheFlags;
            var sw = Stopwatch.StartNew();
            ((IThumbnailCache)cacheObject).GetThumbnail(
                item,
                (uint)size,
                WTS_FLAGS.WTS_FORCEEXTRACTION,
                out sharedBitmap,
                out cacheFlags,
                IntPtr.Zero);
            sw.Stop();
            if (sharedBitmap == null)
            {
                throw new InvalidOperationException("Forced shell extraction returned no shared bitmap.");
            }

            IntPtr bitmap;
            sharedBitmap.GetSharedBitmap(out bitmap);
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Forced shell extraction returned a null bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "shell-force", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (sharedBitmap != null && Marshal.IsComObject(sharedBitmap))
            {
                Marshal.FinalReleaseComObject(sharedBitmap);
            }
            if (item != null && Marshal.IsComObject(item))
            {
                Marshal.FinalReleaseComObject(item);
            }
            if (cacheObject != null && Marshal.IsComObject(cacheObject))
            {
                Marshal.FinalReleaseComObject(cacheObject);
            }
        }
    }

    public static ProbeResult[] ProbeShellBurst(string[] paths, int size)
    {
        var results = new ProbeResult[paths.Length];
        var options = new System.Threading.Tasks.ParallelOptions
        {
            MaxDegreeOfParallelism = Math.Min(8, Math.Max(1, paths.Length))
        };
        System.Threading.Tasks.Parallel.For(0, paths.Length, options, i =>
        {
            results[i] = ProbeShell(paths[i], size);
        });
        return results;
    }

    public static ProbeResult[] ProbeShellBurstBestEffort(string[] paths, int size)
    {
        var results = new ProbeResult[paths.Length];
        var options = new System.Threading.Tasks.ParallelOptions
        {
            MaxDegreeOfParallelism = Math.Min(12, Math.Max(1, paths.Length))
        };
        System.Threading.Tasks.Parallel.For(0, paths.Length, options, i =>
        {
            var sw = Stopwatch.StartNew();
            try
            {
                results[i] = ProbeShell(paths[i], size);
            }
            catch (Exception ex)
            {
                sw.Stop();
                results[i] = new ProbeResult
                {
                    Route = "shell-miss:" + ex.GetType().Name + ":0x" + ex.HResult.ToString("X8"),
                    Width = 0,
                    Height = 0,
                    BitsPerPixel = 0,
                    VisiblePixels = 0,
                    ContentWidth = 0,
                    ContentHeight = 0,
                    Hash = 0,
                    ElapsedMs = sw.ElapsedMilliseconds
                };
            }
        });
        return results;
    }

    public static ProbeResult ProbeDirect(string clsid, string path, int size)
    {
        object instance = null;
        IntPtr bitmap = IntPtr.Zero;
        try
        {
            Type type = Type.GetTypeFromCLSID(new Guid(clsid), true);
            if (type == null)
            {
                throw new InvalidOperationException("Type.GetTypeFromCLSID returned null.");
            }
            instance = Activator.CreateInstance(type);
            if (instance == null)
            {
                throw new InvalidOperationException("Activator.CreateInstance returned null.");
            }
            ((IInitializeWithFile)instance).Initialize(path, 0);
            WTS_ALPHATYPE alpha;
            var sw = Stopwatch.StartNew();
            ((IThumbnailProvider)instance).GetThumbnail((uint)size, out bitmap, out alpha);
            sw.Stop();
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Provider returned a null thumbnail bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "direct", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (instance != null && Marshal.IsComObject(instance))
            {
                Marshal.FinalReleaseComObject(instance);
            }
        }
    }

    public static ProbeResult ProbeDirectFromStream(string clsid, string path, int size)
    {
        object instance = null;
        IStream stream = null;
        IntPtr bitmap = IntPtr.Zero;
        try
        {
            Type type = Type.GetTypeFromCLSID(new Guid(clsid), true);
            if (type == null)
            {
                throw new InvalidOperationException("Type.GetTypeFromCLSID returned null.");
            }
            instance = Activator.CreateInstance(type);
            if (instance == null)
            {
                throw new InvalidOperationException("Activator.CreateInstance returned null.");
            }

            int hr = SHCreateStreamOnFileEx(path, 0x00000020, 0, false, null, out stream);
            if (hr < 0 || stream == null)
            {
                Marshal.ThrowExceptionForHR(hr);
            }

            ((IInitializeWithStream)instance).Initialize(stream, 0);
            WTS_ALPHATYPE alpha;
            var sw = Stopwatch.StartNew();
            ((IThumbnailProvider)instance).GetThumbnail((uint)size, out bitmap, out alpha);
            sw.Stop();
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Stream-initialized provider returned a null thumbnail bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "stream", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (stream != null)
            {
                Marshal.ReleaseComObject(stream);
            }
            if (instance != null && Marshal.IsComObject(instance))
            {
                Marshal.FinalReleaseComObject(instance);
            }
        }
    }

    public static ProbeResult ProbeDirectFromStreamAtOffset(string clsid, string path, int size, long offset)
    {
        object instance = null;
        IStream stream = null;
        IntPtr bitmap = IntPtr.Zero;
        try
        {
            Type type = Type.GetTypeFromCLSID(new Guid(clsid), true);
            if (type == null)
            {
                throw new InvalidOperationException("Type.GetTypeFromCLSID returned null.");
            }
            instance = Activator.CreateInstance(type);
            if (instance == null)
            {
                throw new InvalidOperationException("Activator.CreateInstance returned null.");
            }

            int hr = SHCreateStreamOnFileEx(path, 0x00000020, 0, false, null, out stream);
            if (hr < 0 || stream == null)
            {
                Marshal.ThrowExceptionForHR(hr);
            }

            stream.Seek(offset, 0, IntPtr.Zero);
            ((IInitializeWithStream)instance).Initialize(stream, 0);
            WTS_ALPHATYPE alpha;
            var sw = Stopwatch.StartNew();
            ((IThumbnailProvider)instance).GetThumbnail((uint)size, out bitmap, out alpha);
            sw.Stop();
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Offset stream-initialized provider returned a null thumbnail bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "stream-offset", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (stream != null)
            {
                Marshal.ReleaseComObject(stream);
            }
            if (instance != null && Marshal.IsComObject(instance))
            {
                Marshal.FinalReleaseComObject(instance);
            }
        }
    }

    public static ProbeResult ProbeDirectFromItem(string clsid, string path, int size)
    {
        object instance = null;
        IShellItem item = null;
        IntPtr bitmap = IntPtr.Zero;
        try
        {
            Type type = Type.GetTypeFromCLSID(new Guid(clsid), true);
            if (type == null)
            {
                throw new InvalidOperationException("Type.GetTypeFromCLSID returned null.");
            }
            instance = Activator.CreateInstance(type);
            if (instance == null)
            {
                throw new InvalidOperationException("Activator.CreateInstance returned null.");
            }

            Guid iid = typeof(IShellItem).GUID;
            SHCreateShellItemFromParsingName(path, IntPtr.Zero, ref iid, out item);
            if (item == null)
            {
                throw new InvalidOperationException("SHCreateItemFromParsingName returned a null shell item.");
            }

            ((IInitializeWithItem)instance).Initialize(item, 0);
            WTS_ALPHATYPE alpha;
            var sw = Stopwatch.StartNew();
            ((IThumbnailProvider)instance).GetThumbnail((uint)size, out bitmap, out alpha);
            sw.Stop();
            if (bitmap == IntPtr.Zero)
            {
                throw new InvalidOperationException("Item-initialized provider returned a null thumbnail bitmap.");
            }
            return ProbeOwnedBitmap(bitmap, "item", sw.ElapsedMilliseconds);
        }
        finally
        {
            if (bitmap != IntPtr.Zero)
            {
                DeleteObject(bitmap);
            }
            if (item != null && Marshal.IsComObject(item))
            {
                Marshal.FinalReleaseComObject(item);
            }
            if (instance != null && Marshal.IsComObject(instance))
            {
                Marshal.FinalReleaseComObject(instance);
            }
        }
    }

    public static int HammingDistance(ulong left, ulong right)
    {
        ulong value = left ^ right;
        int count = 0;
        while (value != 0)
        {
            value &= value - 1;
            count++;
        }
        return count;
    }

    private static ProbeResult ProbeOwnedBitmap(IntPtr bitmap, string route, long elapsedMs)
    {
        BITMAP info;
        int bytes = GetObject(bitmap, Marshal.SizeOf<BITMAP>(), out info);
        if (bytes == 0 || info.bmWidth <= 0 || info.bmHeight <= 0)
        {
            throw new InvalidOperationException("Bitmap handle did not expose a valid surface.");
        }

        byte[] pixels = ReadPixels(bitmap, info.bmWidth, info.bmHeight);
        return Analyze(route, elapsedMs, pixels, info.bmWidth, info.bmHeight, info.bmBitsPixel);
    }

    private static byte[] ReadPixels(IntPtr bitmap, int width, int height)
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
        IntPtr hdc = GetDC(IntPtr.Zero);
        if (hdc == IntPtr.Zero)
        {
            throw new InvalidOperationException("GetDC returned a null HDC.");
        }

        try
        {
            int scanLines = GetDIBits(hdc, bitmap, 0, (uint)height, pixels, ref bmi, DIB_RGB_COLORS);
            if (scanLines == 0)
            {
                throw new InvalidOperationException("GetDIBits failed for thumbnail bitmap.");
            }
        }
        finally
        {
            ReleaseDC(IntPtr.Zero, hdc);
        }

        return pixels;
    }

    private static ProbeResult Analyze(string route, long elapsedMs, byte[] pixels, int width, int height, int bitsPerPixel)
    {
        var background = EstimateBackground(pixels, width, height);
        const int diffThreshold = 44;

        int visiblePixels = 0;
        int minX = width;
        int minY = height;
        int maxX = -1;
        int maxY = -1;

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
                if (diff <= diffThreshold)
                {
                    continue;
                }
                visiblePixels++;
                if (x < minX) minX = x;
                if (y < minY) minY = y;
                if (x > maxX) maxX = x;
                if (y > maxY) maxY = y;
            }
        }

        int contentWidth = visiblePixels == 0 ? 0 : (maxX - minX + 1);
        int contentHeight = visiblePixels == 0 ? 0 : (maxY - minY + 1);
        ulong hash = ComputeDifferenceHash(pixels, width, height, background);

        return new ProbeResult
        {
            Route = route,
            Width = width,
            Height = height,
            BitsPerPixel = bitsPerPixel,
            VisiblePixels = visiblePixels,
            ContentWidth = contentWidth,
            ContentHeight = contentHeight,
            Hash = hash,
            ElapsedMs = elapsedMs
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

function Assert-ThumbnailLooksReal {
    param(
        [Parameter(Mandatory = $true)]$Probe,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $pixelCount = [Math]::Max(1, $Probe.Width * $Probe.Height)
    $minimumVisible = [Math]::Max(6, [int]($pixelCount * 0.02))
    $minimumPrimarySpan = [Math]::Max(6, [int]([Math]::Min($Probe.Width, $Probe.Height) * 0.40))
    $minimumSecondarySpan = [Math]::Max(3, [int]([Math]::Min($Probe.Width, $Probe.Height) * 0.10))
    $primarySpan = [Math]::Max($Probe.ContentWidth, $Probe.ContentHeight)
    $secondarySpan = [Math]::Min($Probe.ContentWidth, $Probe.ContentHeight)

    if ($Probe.VisiblePixels -lt $minimumVisible) {
        throw "$Label did not contain enough visible geometry. $($Probe.Summary())"
    }
    if ($primarySpan -lt $minimumPrimarySpan -or $secondarySpan -lt $minimumSecondarySpan) {
        throw "$Label content bounds were too small for a framed thumbnail. $($Probe.Summary())"
    }
}

function Assert-ShellProbeSucceeded {
    param(
        [Parameter(Mandatory = $true)]$Probe,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if ($Probe.Route -like "shell-miss:*") {
        throw "$Label failed through IShellItemImageFactory. $($Probe.Summary())"
    }
    if ($Probe.Width -le 0 -or $Probe.Height -le 0 -or $Probe.BitsPerPixel -le 0) {
        throw "$Label returned an invalid shell bitmap. $($Probe.Summary())"
    }
}

function Assert-ThumbnailPair {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$Size
    )

    $direct = [OccluViewShellThumbnailSmoke]::ProbeDirect($thumbnailProviderClsid, $Path, $Size)
    $stream = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStream($thumbnailProviderClsid, $Path, $Size)
    $streamOffset = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStreamAtOffset($thumbnailProviderClsid, $Path, $Size, 17)
    $item = [OccluViewShellThumbnailSmoke]::ProbeDirectFromItem($thumbnailProviderClsid, $Path, $Size)
    $shell = [OccluViewShellThumbnailSmoke]::ProbeShell($Path, $Size)
    $shellWarm = [OccluViewShellThumbnailSmoke]::ProbeShell($Path, $Size)

    Assert-ThumbnailLooksReal -Probe $direct -Label "$Label direct provider"
    Assert-ThumbnailLooksReal -Probe $stream -Label "$Label stream provider"
    Assert-ThumbnailLooksReal -Probe $streamOffset -Label "$Label offset stream provider"
    Assert-ThumbnailLooksReal -Probe $item -Label "$Label item provider"
    Assert-ThumbnailLooksReal -Probe $shell -Label "$Label shell path"
    Assert-ThumbnailLooksReal -Probe $shellWarm -Label "$Label warm shell path"

    if ($Size -ge 96) {
        $streamDistance = [OccluViewShellThumbnailSmoke]::HammingDistance($direct.Hash, $stream.Hash)
        $streamOffsetDistance = [OccluViewShellThumbnailSmoke]::HammingDistance($direct.Hash, $streamOffset.Hash)
        $itemDistance = [OccluViewShellThumbnailSmoke]::HammingDistance($direct.Hash, $item.Hash)
        $distance = [OccluViewShellThumbnailSmoke]::HammingDistance($direct.Hash, $shell.Hash)
        $maximumDistance = if ($Size -ge 256) { 14 } else { 18 }
        if ($streamDistance -gt $maximumDistance) {
            throw "Thumbnail stream path drifted for $Label at ${Size}px. Direct=$($direct.Summary()) Stream=$($stream.Summary()) Hamming=$streamDistance max=$maximumDistance"
        }
        if ($streamOffsetDistance -gt $maximumDistance) {
            throw "Thumbnail offset-stream path drifted for $Label at ${Size}px. Direct=$($direct.Summary()) OffsetStream=$($streamOffset.Summary()) Hamming=$streamOffsetDistance max=$maximumDistance"
        }
        if ($itemDistance -gt $maximumDistance) {
            throw "Thumbnail item path drifted for $Label at ${Size}px. Direct=$($direct.Summary()) Item=$($item.Summary()) Hamming=$itemDistance max=$maximumDistance"
        }
        if ($distance -gt $maximumDistance) {
            throw "Thumbnail shell path drifted for $Label at ${Size}px. Direct=$($direct.Summary()) Shell=$($shell.Summary()) Hamming=$distance max=$maximumDistance"
        }
        $warmDistance = [OccluViewShellThumbnailSmoke]::HammingDistance($shell.Hash, $shellWarm.Hash)
        if ($warmDistance -gt 6) {
            throw "Warm shell thumbnail drifted for $Label at ${Size}px. Cold=$($shell.Summary()) Warm=$($shellWarm.Summary()) Hamming=$warmDistance"
        }
        Write-Host "$Label $Size px: stream/direct aligned (hamming=$streamDistance) offset-stream/direct aligned (hamming=$streamOffsetDistance) item/direct aligned (hamming=$itemDistance) shell/direct aligned (hamming=$distance) warm-shell-stable (hamming=$warmDistance) direct=$($direct.Summary()) stream=$($stream.Summary()) streamOffset=$($streamOffset.Summary()) item=$($item.Summary()) shell=$($shell.Summary()) shellWarm=$($shellWarm.Summary())"
        return
    }

    Write-Host "$Label $Size px: visible thumbnail direct=$($direct.Summary()) stream=$($stream.Summary()) streamOffset=$($streamOffset.Summary()) item=$($item.Summary()) shell=$($shell.Summary()) shellWarm=$($shellWarm.Summary())"
}

function Assert-TruncatedStreamFallsBackToPlaceholder {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][int]$Size
    )

    # Implementation-independent placeholder contract (the exact art may
    # evolve): a corrupt stream must yield a non-empty, deterministic bitmap
    # that differs from a real render, and two DIFFERENT corrupt files must
    # yield the SAME placeholder art.
    $truncated = New-TruncatedStl -Directory $Directory
    $garbage = New-GarbageStl -Directory $Directory
    $first = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStream($thumbnailProviderClsid, $truncated, $Size)
    $second = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStream($thumbnailProviderClsid, $truncated, $Size)
    $other = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStream($thumbnailProviderClsid, $garbage, $Size)
    $healthy = [OccluViewShellThumbnailSmoke]::ProbeDirectFromStream($thumbnailProviderClsid, (New-SmokeStl -Directory $Directory), $Size)

    if ($first.VisiblePixels -le 0) {
        throw "Corrupt stream produced an empty thumbnail instead of a placeholder. Actual=$($first.Summary())"
    }
    if ($first.Hash -ne $second.Hash -or $first.VisiblePixels -ne $second.VisiblePixels) {
        throw "Corrupt-stream placeholder is not deterministic. First=$($first.Summary()) Second=$($second.Summary())"
    }
    if ($first.Hash -ne $other.Hash -or $first.VisiblePixels -ne $other.VisiblePixels) {
        throw "Two corrupt files produced different placeholders. Truncated=$($first.Summary()) Garbage=$($other.Summary())"
    }
    if ($first.Hash -eq $healthy.Hash -and $first.VisiblePixels -eq $healthy.VisiblePixels) {
        throw "Corrupt stream rendered like a healthy mesh instead of the placeholder. Actual=$($first.Summary()) Healthy=$($healthy.Summary())"
    }

    Write-Host "corrupt stream placeholder $Size px: deterministic, shared across corrupt inputs, distinct from real renders. actual=$($first.Summary())"
}

function New-SmokeCases {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][int[]]$RequestedSizes
    )

    @(
        [pscustomobject]@{
            Label = "stl"
            Path = New-SmokeStl -Directory $Directory
            Sizes = $RequestedSizes
        }
        [pscustomobject]@{
            Label = "ply"
            Path = New-SmokePly -Directory $Directory
            Sizes = @(96, 256)
        }
        [pscustomobject]@{
            Label = "obj"
            Path = New-SmokeObj -Directory $Directory
            Sizes = @(96, 256)
        }
        [pscustomobject]@{
            Label = "obj-800kb"
            Path = New-NoisyObj -Directory $Directory
            Sizes = @(96, 256)
        }
        [pscustomobject]@{
            Label = "hps"
            Path = New-SmokeHps -Directory $Directory
            Sizes = @(96, 256)
        }
        [pscustomobject]@{
            Label = "hps-legacy"
            Path = New-SmokeLegacyHps -Directory $Directory
            Sizes = @(96, 256)
        }
    )
}

function Assert-MixedFolderBurst {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $burstDirectory = Join-Path $Directory "occluview-thumbnail-mixed"
    New-Item -ItemType Directory -Path $burstDirectory -Force | Out-Null

    $sources = @(
        New-SmokeStl -Directory $burstDirectory
        New-SmokePly -Directory $burstDirectory
        New-SmokeObj -Directory $burstDirectory
        New-NoisyObj -Directory $burstDirectory -Name "occluview-thumbnail-noisy-800kb.obj"
        New-SmokeHps -Directory $burstDirectory
        New-SmokeLegacyHps -Directory $burstDirectory
    )

    $requests = New-Object System.Collections.Generic.List[object]
    $addRequest = {
        param([string]$Path, [bool]$Is3d)
        $requests.Add([pscustomobject]@{
            Path = $Path
            Is3d = $Is3d
        })
    }

    $noiseText = @{
        "case-notes.txt" = "noise file"
        "order.html" = "<!doctype html><title>scan case</title><body>case</body>"
        "measurements.csv" = "id,value`n1,2"
        "manifest.json" = "{""kind"":""case"",""count"":2}"
        "readme.md" = "# case"
        "metadata.xml" = "<case><id>1</id></case>"
    }
    foreach ($entry in $noiseText.GetEnumerator()) {
        for ($copy = 0; $copy -lt 3; $copy++) {
            $baseName = [IO.Path]::GetFileNameWithoutExtension($entry.Key)
            $extension = [IO.Path]::GetExtension($entry.Key)
            $noisePath = Join-Path $burstDirectory ("{0}-{1:D2}{2}" -f $baseName, $copy, $extension)
            [IO.File]::WriteAllText($noisePath, $entry.Value, [Text.UTF8Encoding]::new($false))
            & $addRequest $noisePath $false
        }
    }
    for ($copy = 0; $copy -lt 3; $copy++) {
        $pngPath = Join-Path $burstDirectory ("snapshot-{0:D2}.png" -f $copy)
        [IO.File]::WriteAllBytes($pngPath, [Convert]::FromBase64String("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="))
        & $addRequest $pngPath $false
        $binPath = Join-Path $burstDirectory ("preview-{0:D2}.bin" -f $copy)
        [IO.File]::WriteAllBytes($binPath, [byte[]](0, 1, 2, 3, 4, 5))
        & $addRequest $binPath $false
    }

    for ($i = 0; $i -lt 96; $i++) {
        $source = $sources[$i % $sources.Count]
        $extension = [IO.Path]::GetExtension($source).ToLowerInvariant()
        $target = Join-Path $burstDirectory ("mixed-{0:D2}{1}" -f $i, $extension)
        Copy-Item -LiteralPath $source -Destination $target -Force
        & $addRequest $target $true
    }

    $allPaths = [string[]]($requests | ForEach-Object { $_.Path })
    $burstSizes = @(96, 256)
    foreach ($size in $burstSizes) {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $results = [OccluViewShellThumbnailSmoke]::ProbeShellBurstBestEffort($allPaths, $size)
        $sw.Stop()

        if ($results.Length -ne $requests.Count) {
            throw "Mixed folder burst at ${size}px returned $($results.Length) results for $($requests.Count) requests."
        }

        for ($i = 0; $i -lt $results.Length; $i++) {
            $request = $requests[$i]
            $result = $results[$i]
            if ($request.Is3d -and $result.Route -like "shell-miss:*") {
                try {
                    $retry = [OccluViewShellThumbnailSmoke]::ProbeShellForced($request.Path, $size)
                } catch {
                    throw "Mixed folder shell thumbnail failed for ${size}px $([IO.Path]::GetFileName($request.Path)) and did not recover through forced shell extraction. First=$($result.Summary()) Retry=$($_.Exception.GetType().Name): $($_.Exception.Message)"
                }
                Write-Host "mixed folder forced extraction recovered ${size}px $([IO.Path]::GetFileName($request.Path)): first=$($result.Route) retry=$($retry.Summary())"
                $result = $retry
            }
            if (-not $request.Is3d) {
                continue
            }
            Assert-ThumbnailLooksReal -Probe $result -Label ("mixed folder shell thumbnail ${size}px " + [IO.Path]::GetFileName($request.Path))
        }

        $slowest = ($results | Where-Object { $_.ElapsedMs -gt 0 } | Measure-Object -Property ElapsedMs -Maximum).Maximum
        $requestCount = $results.Length
        $threedCount = ($requests | Where-Object { $_.Is3d }).Count
        $noiseCount = $requestCount - $threedCount
        Write-Host "mixed folder burst ${size}px: $requestCount shell requests / $threedCount 3D thumbnails / $noiseCount noise neighbors in $($sw.ElapsedMilliseconds)ms slowest=$slowest ms"
        if ($sw.ElapsedMilliseconds -gt 45000) {
            throw "Mixed folder thumbnail burst at ${size}px was too slow: $($sw.ElapsedMilliseconds)ms"
        }
    }
}

$tempDirectory = $null
try {
    if ([string]::IsNullOrWhiteSpace($SamplePath)) {
        $tempDirectory = Join-Path $env:TEMP ("occluview-thumbnail-smoke-" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $tempDirectory -Force | Out-Null
        $cases = New-SmokeCases -Directory $tempDirectory -RequestedSizes $Sizes
    } else {
        $resolvedPath = (Resolve-Path $SamplePath).Path
        $cases = @(
            [pscustomobject]@{
                Label = [IO.Path]::GetExtension($resolvedPath).TrimStart(".").ToLowerInvariant()
                Path = $resolvedPath
                Sizes = $Sizes
            }
        )
    }

    foreach ($case in $cases) {
        foreach ($size in $case.Sizes) {
            if ($size -lt 16 -or $size -gt 1024) {
                throw "Thumbnail smoke size must be in the Windows shell thumbnail range: $size"
            }
            Assert-ThumbnailPair -Label $case.Label -Path $case.Path -Size $size
        }
    }
    if ($null -ne $tempDirectory) {
        Assert-MixedFolderBurst -Directory $tempDirectory
        Assert-TruncatedStreamFallsBackToPlaceholder -Directory $tempDirectory -Size 32
    }
} finally {
    if ($null -ne $tempDirectory -and (Test-Path $tempDirectory)) {
        Remove-Item $tempDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
