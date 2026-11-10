<#
.SYNOPSIS
    Reads the icon Windows shows for a program: the first icon group among its resources.

.DESCRIPTION
    Dot-sourced by check-kerf-windows.ps1 and check-kerf-windows-installer.ps1. Explorer,
    the taskbar and the Start menu draw a program with the first icon group in it, picking
    the picture nearest the size they need.

    Get-KerfProgramIcon gives one object for each picture in that group, with its Side in
    pixels and its Data, the bytes of the picture (a bitmap, or a PNG at 256 pixels); none
    when the program has no icon.

    Assert-KerfProgramIcon throws unless the program has an icon with a picture at each of
    16, 32, 48 and 256 pixels, the sizes a title bar, a taskbar, a large icon and a tile ask
    for, and gives the pictures back.
#>

# Both checks read this file, and the installer's runs the package's.
if (-not ('KerfCheck.Icons' -as [type])) {
    Add-Type -Namespace KerfCheck -Name Icons -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
static extern IntPtr LoadLibraryExW(string file, IntPtr reserved, uint flags);
[DllImport("kernel32.dll")]
static extern bool FreeLibrary(IntPtr module);
delegate bool NameCallback(IntPtr module, IntPtr type, IntPtr name, IntPtr parameter);
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
static extern bool EnumResourceNamesW(IntPtr module, IntPtr type, NameCallback callback, IntPtr parameter);
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
[DllImport("kernel32.dll")]
static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
[DllImport("kernel32.dll")]
static extern IntPtr LockResource(IntPtr data);
[DllImport("kernel32.dll")]
static extern uint SizeofResource(IntPtr module, IntPtr resource);

static byte[] Read(IntPtr module, IntPtr name, int type) {
    IntPtr resource = FindResourceW(module, name, (IntPtr)type);
    if (resource == IntPtr.Zero) return null;
    IntPtr data = LockResource(LoadResource(module, resource));
    if (data == IntPtr.Zero) return null;
    byte[] bytes = new byte[SizeofResource(module, resource)];
    System.Runtime.InteropServices.Marshal.Copy(data, bytes, 0, bytes.Length);
    return bytes;
}

// The pictures of the first icon group in the file, as side, bytes, side, bytes...;
// empty when it has no icon group.
public static object[] FirstGroup(string file) {
    // LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE: read, never run.
    IntPtr module = LoadLibraryExW(file, IntPtr.Zero, 0x2 | 0x20);
    if (module == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
    try {
        byte[] group = null;
        // RT_GROUP_ICON is 14. The name is only good inside the callback.
        EnumResourceNamesW(module, (IntPtr)14, (m, type, name, parameter) => {
            group = Read(m, name, 14);
            return false;
        }, IntPtr.Zero);
        var pictures = new System.Collections.Generic.List<object>();
        if (group == null || group.Length < 6) return pictures.ToArray();
        int count = BitConverter.ToUInt16(group, 4);
        for (int i = 0; i < count && 6 + 14 * (i + 1) <= group.Length; i++) {
            int at = 6 + 14 * i;
            // A side of 256 is written as 0. RT_ICON is 3.
            int side = group[at] == 0 ? 256 : group[at];
            byte[] picture = Read(module, (IntPtr)BitConverter.ToUInt16(group, at + 12), 3);
            pictures.Add(side);
            pictures.Add(picture ?? new byte[0]);
        }
        return pictures.ToArray();
    } finally {
        FreeLibrary(module);
    }
}
'@
}

function Get-KerfProgramIcon([string]$Program) {
    $flat = [KerfCheck.Icons]::FirstGroup((Resolve-Path $Program).Path)
    $pictures = @()
    for ($i = 0; $i + 1 -lt $flat.Count; $i += 2) {
        $pictures += [pscustomobject]@{ Side = [int]$flat[$i]; Data = [byte[]]$flat[$i + 1] }
    }
    return , $pictures
}

function Assert-KerfProgramIcon([string]$Program) {
    $name = Split-Path $Program -Leaf
    $pictures = Get-KerfProgramIcon $Program
    if ($pictures.Count -eq 0) { throw "$name has no icon among its resources" }
    foreach ($side in 16, 32, 48, 256) {
        $picture = $pictures | Where-Object { $_.Side -eq $side } | Select-Object -First 1
        if (-not $picture -or $picture.Data.Length -eq 0) {
            throw "$name's icon has no $side pixel picture (it has $(($pictures.Side | Sort-Object -Unique) -join ', '))"
        }
    }
    return , $pictures
}
