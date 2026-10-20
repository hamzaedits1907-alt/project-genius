# Regenerates the Android launcher icons and the iOS app icon from src-tauri/icons/icon.png (the Genius logo).
# Run from the repository root:  powershell -File scripts/make-mobile-icons.ps1
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Image]::FromFile((Resolve-Path 'src-tauri/icons/icon.png'))

function Save-Scaled($path, [int]$size, [double]$logoScale, $background) {
  $bmp = New-Object System.Drawing.Bitmap $size, $size
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'HighQuality'; $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'
  if ($background) { $g.Clear($background) } else { $g.Clear([System.Drawing.Color]::Transparent) }
  $d = [int]($size * $logoScale)
  $o = [int](($size - $d) / 2)
  $g.DrawImage($src, $o, $o, $d, $d)
  $g.Dispose()
  New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

$dark = [System.Drawing.Color]::FromArgb(255, 11, 11, 18)
$densities = @{ mdpi = 48; hdpi = 72; xhdpi = 96; xxhdpi = 144; xxxhdpi = 192 }
foreach ($k in $densities.Keys) {
  $s = $densities[$k]
  $dir = "android/app/src/main/res/mipmap-$k"
  Save-Scaled "$dir/ic_launcher.png" $s 0.82 $dark
  Save-Scaled "$dir/ic_launcher_round.png" $s 0.78 $dark
  # adaptive icon foreground: 108 dp canvas, the logo inside the 66 % safe zone
  Save-Scaled "$dir/ic_launcher_foreground.png" ([int]($s * 108 / 48)) 0.60 $null
}
Save-Scaled 'ios/App/App/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png' 1024 0.80 $dark
$src.Dispose()

$bg = 'android/app/src/main/res/values/ic_launcher_background.xml'
(Get-Content $bg -Raw) -replace '#FFFFFF', '#0B0B12' | Set-Content $bg -NoNewline
Write-Host 'Mobile icons written.'
