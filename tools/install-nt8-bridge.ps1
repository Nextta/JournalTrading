#requires -Version 5.1
<#
.SYNOPSIS
    Instala el AddOn de JournalTrading en NinjaTrader 8 y, opcionalmente, lo compila.

.DESCRIPTION
    Copia nt8\JournalTradingBridge.cs a la carpeta Custom de NinjaTrader 8, añade
    el archivo al proyecto NinjaTrader.Custom.csproj, genera el archivo de
    configuración con la API key y compila con MSBuild.

    Cierra NinjaTrader antes de ejecutar el script: el DLL NinjaTrader.Custom.dll
    está bloqueado mientras la aplicación está abierta.

.PARAMETER ApiKey
    API key de JournalTrading (Ajustes > Seguridad). Si se omite, se conserva la
    configuración existente y puedes generarla después desde la app.

.PARAMETER Port
    Puerto HTTP local de JournalTrading. Por defecto 4477.

.PARAMETER NinjaTraderDir
    Carpeta de datos de NinjaTrader 8. Por defecto Documents\NinjaTrader 8.

.PARAMETER NoBuild
    Copia el código y la configuración, pero no compila. Después abre el editor
    de NinjaScript en NinjaTrader y pulsa F5.

.PARAMETER NoConfig
    No genera JournalTradingBridge.json.
#>
param(
    [string]$ApiKey = $env:JOURNAL_TRADING_API_KEY,
    [int]$Port = 4477,
    [string]$NinjaTraderDir = (Join-Path $env:USERPROFILE 'Documents\NinjaTrader 8'),
    [switch]$NoBuild,
    [switch]$NoConfig
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$sourceFile = Join-Path $repoRoot 'nt8\JournalTradingBridge.cs'
$customDir = Join-Path $NinjaTraderDir 'bin\Custom'
$addOnsDir = Join-Path $customDir 'AddOns'
$targetFile = Join-Path $addOnsDir 'JournalTradingBridge.cs'
$projectFile = Join-Path $customDir 'NinjaTrader.Custom.csproj'
$configFile = Join-Path $customDir 'JournalTradingBridge.json'

if (-not (Test-Path $sourceFile)) {
    throw "No se encontró el código del AddOn: $sourceFile"
}
if (-not (Test-Path $customDir)) {
    throw "No se encontró la carpeta Custom de NinjaTrader 8: $customDir. Abre NinjaTrader una vez o indica -NinjaTraderDir."
}
if (-not (Test-Path $projectFile)) {
    throw "No se encontró NinjaTrader.Custom.csproj en $customDir."
}

$ninjaProcess = Get-Process -Name 'NinjaTrader' -ErrorAction SilentlyContinue
if ($ninjaProcess -and -not $NoBuild) {
    throw 'NinjaTrader está en ejecución. Ciérralo antes de compilar (el DLL está bloqueado) o usa -NoBuild.'
}

New-Item -ItemType Directory -Path $addOnsDir -Force | Out-Null
Copy-Item -LiteralPath $sourceFile -Destination $targetFile -Force
Write-Host "AddOn copiado a $targetFile"

# Añadir el archivo al proyecto sin duplicarlo.
$projectText = Get-Content -LiteralPath $projectFile -Raw
$compileEntry = 'AddOns\JournalTradingBridge.cs'
if ($projectText -notmatch [regex]::Escape($compileEntry)) {
    $backup = "$projectFile.journaltrading.bak"
    if (-not (Test-Path $backup)) {
        Copy-Item -LiteralPath $projectFile -Destination $backup
    }
    $insert = "  <ItemGroup>`r`n    <Compile Include=`"$compileEntry`" />`r`n  </ItemGroup>`r`n</Project>"
    $projectText = $projectText -replace '</Project>\s*$', $insert
    # Escribir en un temporal y reemplazar: algunos editores mantienen el csproj abierto.
    $tempProject = "$projectFile.journaltrading.new"
    Set-Content -LiteralPath $tempProject -Value $projectText -Encoding UTF8
    Move-Item -LiteralPath $tempProject -Destination $projectFile -Force
    Write-Host 'Archivo añadido a NinjaTrader.Custom.csproj'
} else {
    Write-Host 'El proyecto ya incluye JournalTradingBridge.cs'
}

if (-not $NoConfig) {
    if ([string]::IsNullOrWhiteSpace($ApiKey)) {
        Write-Warning 'No se indicó API key. Genera JournalTradingBridge.json desde JournalTrading > Ajustes > Puente NinjaTrader 8.'
    } else {
        $config = [ordered]@{
            apiUrl           = "http://127.0.0.1:$Port"
            apiKey           = $ApiKey
            heartbeatSeconds = 5
            syncSeconds      = 15
            historyDays      = 0
            verbose          = $true
        }
        $config | ConvertTo-Json | Set-Content -LiteralPath $configFile -Encoding UTF8
        Write-Host "Configuración escrita en $configFile"
    }
}

if ($NoBuild) {
    Write-Host ''
    Write-Host 'Código instalado. Abre NinjaTrader, ve a Herramientas > Editor de NinjaScript y pulsa F5 para compilar.'
    return
}

$msbuild = Get-Command 'MSBuild.exe' -ErrorAction SilentlyContinue
if (-not $msbuild) {
    $msbuild = Get-ChildItem 'C:\Program Files\Microsoft Visual Studio' -Recurse -Filter 'MSBuild.exe' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -notmatch '\\amd64\\' } |
        Select-Object -First 1
}
if (-not $msbuild) {
    $msbuild = Get-ChildItem 'C:\Program Files (x86)\Microsoft Visual Studio' -Recurse -Filter 'MSBuild.exe' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -notmatch '\\amd64\\' } |
        Select-Object -First 1
}
if (-not $msbuild) {
    throw 'No se encontró MSBuild. Instala Visual Studio Build Tools o compila desde el editor de NinjaScript.'
}

$msbuildPath = if ($msbuild -is [System.Management.Automation.ApplicationInfo]) { $msbuild.Source } else { $msbuild.FullName }
Write-Host "Compilando con $msbuildPath"
& $msbuildPath $projectFile /t:Build /p:Configuration=Release /p:Platform=x64 /v:minimal
if ($LASTEXITCODE -ne 0) {
    throw "La compilación falló con código $LASTEXITCODE. Revisa la salida anterior."
}

# MSBuild deja el DLL en bin\Release. NinjaTrader carga el DLL de la raíz de Custom.
$builtDll = Join-Path $customDir 'bin\Release\NinjaTrader.Custom.dll'
$rootDll = Join-Path $customDir 'NinjaTrader.Custom.dll'
if (-not (Test-Path $builtDll)) {
    throw "La compilación terminó pero no se encontró $builtDll."
}
Copy-Item -LiteralPath $builtDll -Destination $rootDll -Force
$builtPdb = Join-Path $customDir 'bin\Release\NinjaTrader.Custom.pdb'
if (Test-Path $builtPdb) {
    Copy-Item -LiteralPath $builtPdb -Destination (Join-Path $customDir 'NinjaTrader.Custom.pdb') -Force
}
Write-Host "DLL copiado a $rootDll"

Write-Host ''
Write-Host 'AddOn compilado correctamente. Abre NinjaTrader 8 para empezar a sincronizar.'
