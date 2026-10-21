# aexlo 28e9f9a stores pixel formats as u32. On Windows, after-effects-sys
# defines PrPixelFormat as c_int, so those two call sites do not compile.
# The casts are applied once to the cargo git checkout of that revision.
$ErrorActionPreference = "Stop"
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$root = Join-Path $cargoHome "git\checkouts"
if (-not (Test-Path $root)) { exit 0 }
Get-ChildItem $root -Directory -Filter "aexlo-*" | ForEach-Object {
    Get-ChildItem $_.FullName -Recurse -Filter "pixel_format.rs" -File | ForEach-Object {
        $text = [IO.File]::ReadAllText($_.FullName)
        $next = $text.Replace(
            "i.add_supported_pixel_format(pixelFormat)",
            "i.add_supported_pixel_format(pixelFormat as u32)"
        ).Replace(
            "4 * depth.bytes(), pixelFormat)",
            "4 * depth.bytes(), pixelFormat as u32)"
        )
        if ($next -ne $text) {
            [IO.File]::WriteAllText($_.FullName, $next)
        }
    }
}
