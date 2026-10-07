//! H-187 review (0.17.4): paths a PowerShell line computes rather than
//! names. A destructive cmdlet fed `$_` acts on whatever the pipeline lists
//! (M1); .NET's `[IO.File]`/`[IO.Directory]` write, delete, move and copy
//! (F1); `'C:\Users\' + 'x'` is the path it spells (F2).

use super::guard_powershell::{allowed, ps, refused};

#[test]
fn pipeline_paths_are_judged_like_the_no_path_case() {
    refused(&[
        "gci ~/Documents -Recurse | % { Remove-Item $_.FullName }",
        "Get-ChildItem ~\\Documents -Recurse | ForEach-Object { Remove-Item $_.FullName -Force }",
        "Get-ChildItem ~\\Documents | ForEach-Object -Process { Remove-Item -LiteralPath $PSItem.FullName }",
        "gci ~\\Documents | foreach { rm $_ }",
        "gci ~\\Documents | % { Remove-Item \"$($_.FullName)\" }",
        "(gci ~\\Documents).ForEach({ Remove-Item $_ })",
        "foreach ($f in Get-ChildItem ~\\Documents) { Remove-Item $f.FullName }",
        "$files = gci /Users/me/Pictures; foreach ($f in $files) { Remove-Item $f -Recurse }",
        "gci ~\\Documents -PipelineVariable f | % { Remove-Item $f }",
        "gci ~\\Documents -pv:f | % { Remove-Item $f }",
        "gci ~\\Documents | Where-Object { $_.Name -like '*.md' } | % { Remove-Item $_ }",
        "gci ~\\Documents | ? { Remove-Item $_ }",
        "gci ~\\Documents | Where-Object { $_.Length -gt 0 } | Remove-Item",
        "gci ~\\Documents | % { Move-Item $_ . }",
        "gci ~\\Documents | % { Copy-Item notes.md -Destination $_ }",
        "gci ~\\Documents | % { Set-Content $_.FullName '' }",
        "gci ~\\Documents | % { Rename-Item $_ x.old }",
        "Set-Location ~; gci | % { Remove-Item $_ -Recurse }",
        "gci ~\\Documents | % { [IO.File]::Delete($_.FullName) }",
    ]);
    let why = ps("gci ~/Documents -Recurse | % { Remove-Item $_.FullName }").unwrap_or_default();
    assert!(why.contains("`Remove-Item`"), "{why}");
    assert!(why.contains("outside your own folders"), "{why}");
}

#[test]
fn dotnet_file_methods_are_judged_by_their_paths() {
    refused(&[
        "[IO.File]::Delete('/Users/me/notes.md')",
        "[System.IO.File]::Delete(\"$HOME\\notes.md\")",
        "[IO.Directory]::Delete('/Users/me/Documents', $true)",
        "[System.IO.Directory]::Delete(\"$env:USERPROFILE\\Documents\", $true)",
        "[IO.File]::WriteAllText('/etc/hosts', 'x')",
        "[IO.File]::WriteAllBytes(\"$HOME\\x.exe\", $bytes)",
        "[io.file]::AppendAllText('~\\.bashrc', 'x')",
        "[IO.File]::WriteAllTextAsync('/Users/me/x', 'y')",
        "[IO.File]::Move('notes.md', '/Users/me/Documents/notes.md')",
        "[IO.File]::Move('/Users/me/notes.md', 'x.md')",
        "[IO.File]::Copy('evil.ps1', '/Users/me/Documents/evil.ps1', $true)",
        "[IO.Directory]::Move('/Users/me/Pictures', 'pics')",
        "[IO.File]::Replace('a.txt', '/Users/me/b.txt', $null)",
        "[IO.File]::CreateSymbolicLink('.claude', '/tmp/x')",
        "$null = [IO.File]::Delete('/Users/me/x')",
        "[void][IO.File]::Delete('/Users/me/x')",
        "if (Test-Path x) { [IO.File]::Delete('/Users/me/x') }",
        "$p = '/Users/me/x'; [IO.File]::Delete($p)",
    ]);
    let why = ps("[IO.File]::Delete('/Users/me/notes.md')").unwrap_or_default();
    assert!(why.contains("`[IO.File]::Delete`"), "{why}");
}

#[test]
fn string_concatenation_is_folded_before_paths_are_judged() {
    refused(&[
        "Remove-Item ('/Users/me/' + 'Documents') -Recurse",
        "Remove-Item ('/Users/' +'me' + '/x')",
        "Remove-Item ('/Users/me/' + \"Documents\")",
        "$p = '/Users/me/' + 'Pictures'; Remove-Item $p -Recurse",
        "Remove-Item ($HOME + '\\Documents') -Recurse",
        "Remove-Item ($env:USERPROFILE + '\\' + 'Documents')",
        "Set-Content ('~\\' + '.bashrc') 'x'",
        "Get-Content ($HOME + '\\.ssh\\id_rsa')",
        "[IO.File]::Delete($HOME + '\\notes.md')",
        "$d = 'Documents'; Remove-Item (\"$HOME/$d\" + '/x')",
    ]);
}

#[test]
fn everyday_pipelines_dotnet_and_concatenation_pass() {
    allowed(&[
        "Get-ChildItem -Recurse *.tmp | ForEach-Object { Remove-Item $_.FullName }",
        "gci build | % { Remove-Item $_ -Recurse }",
        "foreach ($f in Get-ChildItem *.log) { Remove-Item $f }",
        "Get-ChildItem | Where-Object { $_.Length -eq 0 } | Remove-Item",
        "Get-ChildItem /tmp/x | % { Remove-Item $_ }",
        "Get-Process | Where-Object { $_.CPU -gt 100 } | Select-Object Name",
        "Get-ChildItem ~\\Documents | ForEach-Object { $_.Name }",
        "gci -Recurse src | % { Get-Content $_.FullName } | Measure-Object -Line",
        "[IO.File]::ReadAllText('README.md')",
        "[IO.File]::ReadAllText('/Users/me/Documents/notes.md')",
        "[IO.File]::WriteAllText('notes.md', 'hi')",
        "[IO.File]::WriteAllText(\"$PWD\\notes.md\", 'hi')",
        "[System.IO.File]::Copy('a.txt', 'b.txt', $true)",
        "[IO.File]::Delete('/tmp/x.txt')",
        "[IO.Directory]::CreateDirectory('build')",
        "[IO.Path]::Combine($HOME, 'x')",
        "[IO.File]::Exists('/Users/me/x')",
        "Write-Output ('a' + 'b')",
        "$n = 1 + 2",
        "Write-Host ('Total: ' + $count)",
        "$p = 'build\\' + 'x'; Remove-Item $p -Recurse",
        "Set-Content ('notes' + '.md') 'x'",
    ]);
}
