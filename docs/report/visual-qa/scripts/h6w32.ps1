Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class D {
[DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string cls, string name);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}
"@
$c=[D]::GetDlgItem([IntPtr]9244620, 2); Write-Output ("cancel btn " + $c); [void][D]::PostMessage($c, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero); Start-Sleep -Milliseconds 800
$h=[IntPtr]16585408; $combo=[D]::GetDlgItem($h, 1148); Write-Output ("combo " + $combo)
$edit=[D]::FindWindowEx($combo, [IntPtr]::Zero, "Edit", $null); if ($edit -eq [IntPtr]::Zero) { $cbex=[D]::FindWindowEx($combo, [IntPtr]::Zero, "ComboBox", $null); $edit=[D]::FindWindowEx($cbex, [IntPtr]::Zero, "Edit", $null) }
Write-Output ("edit " + $edit); [void][D]::SendMessage($edit, 0x000C, [IntPtr]::Zero, "C:\Users\lsu10\simrep-gui-test\visual-qa\tmp\h6check.vti")
$ok=[D]::GetDlgItem($h, 1); Write-Output ("save btn " + $ok); [void][D]::PostMessage($ok, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
