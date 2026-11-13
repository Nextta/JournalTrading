#requires -Version 5.1
<#
.SYNOPSIS
    Compila e instala el indicador JournalTradingBridge en ATAS X.

.DESCRIPTION
    Compila atas\JournalTradingBridge con el SDK de .NET contra los ensamblados de ATAS X,
    copia el DLL a %APPDATA%\ATAS X\Indicators y, si se indica la API key, genera
    %APPDATA%\ATAS X\JournalTradingBridge.json para que el indicador la lea.

    El indicador es opcional: JournalTrading ya lee el historial de la base de datos local de
    ATAS. El indicador añade datos en tiempo real y velas para los gráficos.

.PARAMETER ApiKey
    API key de JournalTrading (Ajustes > Seguridad). Si se omite, se conserva la configuración
    existente; también puedes generarla desde la app.

.PARAMETER Port
    Puerto HTTP local de JournalTrading. Por defecto 4477.

.PARAMETER AtasDir
    Carpeta de instalación de ATAS X. Por defecto C:\Program Files\ATAS X.

.PARAMETER AtasDataDir
    Carpeta de datos de ATAS X. Por defecto %APPDATA%\ATAS X.
#>
param(
    [string]$ApiKey = $env:JOURNAL_TRADING_API_KEY,
    [int]$Port = 4477,
    [string]$AtasDir = 'C:\Program Files\ATAS X',
    [string]$AtasDataDir = (Join-Path $env:APPDATA 'ATAS X')
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$project = Join-Path $repoRoot 'atas\JournalTradingBridge\JournalTradingBridge.csproj'
$indicatorsDir = Join-Path $AtasDataDir 'Indicators'
$configFile = Join-Path $AtasDataDir 'JournalTradingBridge.json'
$outputDir = Join-Path $repoRoot 'atas\JournalTradingBridge\bin\install'

if (-not (Test-Path (Join-Path $AtasDir 'ATAS.Indicators.dll'))) {
    throw "No se encontró ATAS X en $AtasDir. Indica la carpeta con -AtasDir."
}
if (-not (Test-Path $AtasDataDir)) {
    throw "No se encontró la carpeta de datos de ATAS X: $AtasDataDir. Abre ATAS una vez o indica -AtasDataDir."
}
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw 'No se encontró el SDK de .NET (dotnet). Instala el SDK de .NET 10.'
}

Write-Host "Compilando $project"
& dotnet build $project -c Release -nologo -v q "-p:AtasDir=$AtasDir" -o $outputDir
if ($LASTEXITCODE -ne 0) {
    throw "La compilación falló con código $LASTEXITCODE. Revisa la salida anterior."
}

New-Item -ItemType Directory -Path $indicatorsDir -Force | Out-Null
$dll = Join-Path $outputDir 'JournalTradingBridge.dll'
$target = Join-Path $indicatorsDir 'JournalTradingBridge.dll'
try {
    Copy-Item -LiteralPath $dll -Destination $target -Force
} catch {
    throw "No se pudo copiar el DLL a $target. Si ATAS lo tiene bloqueado, ciérralo y vuelve a ejecutar el script. $($_.Exception.Message)"
}
Write-Host "Indicador copiado a $target"

if ([string]::IsNullOrWhiteSpace($ApiKey)) {
    if (-not (Test-Path $configFile)) {
        Write-Warning 'No se indicó API key. Genera la configuración desde JournalTrading > Ajustes > ATAS X o escríbela en los ajustes del indicador.'
    }
} else {
    $config = [ordered]@{
        apiUrl = "http://127.0.0.1:$Port"
        apiKey = $ApiKey
    }
    $json = $config | ConvertTo-Json
    [System.IO.File]::WriteAllText($configFile, $json, (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "Configuración escrita en $configFile"
}

Write-Host ''
Write-Host 'Indicador instalado. En ATAS, añade "JournalTrading Bridge" (categoría JournalTrading) a un gráfico de la cuenta que quieras sincronizar en tiempo real.'
