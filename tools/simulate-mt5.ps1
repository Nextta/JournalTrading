param(
    [string]$ApiKey = $env:PLAN_TRADING_API_KEY,
    [int]$Port = 4477,
    [long]$Login = 990001,
    [string]$Server = "Simulator-Demo",
    [int]$OffsetSeconds = 7200,
    [int]$HeartbeatSeconds = 5,
    [int]$SyncSeconds = 15
)

if ([string]::IsNullOrWhiteSpace($ApiKey)) {
    throw "Falta la API key. Usa -ApiKey <valor> o define PLAN_TRADING_API_KEY. La encuentras en Ajustes > API key."
}

$Base = "http://127.0.0.1:$Port"
$Headers = @{ "X-Api-Key" = $ApiKey }

function Get-ServerNow {
    return [DateTime]::UtcNow.AddSeconds($OffsetSeconds)
}

function Format-ServerTime([datetime]$Value) {
    return $Value.ToString("yyyy.MM.dd HH:mm:ss")
}

function Format-IsoTime([datetime]$Value) {
    return $Value.ToString("yyyy-MM-ddTHH:mm:ss")
}

function To-Epoch([datetime]$Value) {
    return [DateTimeOffset]::new($Value).ToUnixTimeSeconds()
}

function From-Epoch([long]$Value) {
    return [DateTimeOffset]::FromUnixTimeSeconds($Value).UtcDateTime
}

function Invoke-Bridge {
    param(
        [string]$Path,
        [hashtable]$Payload
    )

    $json = $Payload | ConvertTo-Json -Depth 12 -Compress
    try {
        return Invoke-RestMethod -Uri "$Base$Path" -Method Post -Headers $Headers `
            -ContentType "application/json; charset=utf-8" -Body $json -TimeoutSec 20
    }
    catch {
        Write-Warning ("[http] " + $Path + " -> " + $_.Exception.Message)
        return $null
    }
}

function Build-HeartbeatPayload {
    $now = Get-ServerNow
    return @{
        login = $Login
        server = $Server
        display_name = "Cuenta Simulada"
        company = "Plan de Trading Simulator"
        currency = "USD"
        leverage = 100
        broker_utc_offset_seconds = $OffsetSeconds
        balance = 10000.0
        equity = 10012.5
        margin = 254.0
        free_margin = 9758.5
        ea_version = "sim-1.0.0"
        terminal_build = 5000
        terminal_data_path = "C:\Simulator\Terminal"
        server_time = (Format-IsoTime $now)
    }
}

function Build-SyncPayload {
    $now = Get-ServerNow
    $yesterday = $now.AddDays(-1)
    $openTime = $now.AddHours(-2)

    $symbols = @(
        @{
            symbol = "EURUSD"; description = "Euro vs US Dollar"; digits = 5
            tick_size = 0.00001; tick_value = 1.0; contract_size = 100000.0
            currency_profit = "USD"
        },
        @{
            symbol = "XAUUSD"; description = "Gold vs US Dollar"; digits = 2
            tick_size = 0.01; tick_value = 1.0; contract_size = 100.0
            currency_profit = "USD"
        },
        @{
            symbol = "GBPUSD"; description = "Great Britain Pound vs US Dollar"; digits = 5
            tick_size = 0.00001; tick_value = 1.0; contract_size = 100000.0
            currency_profit = "USD"
        }
    )

    $positions = @(
        @{
            ticket = 500003; symbol = "GBPUSD"; type = 0; volume = 0.20
            price_open = 1.27000; sl = $null; tp = $null
            profit = 12.5; swap = -0.3; commission = 0.0
            magic = 0; comment = "sim-open"; time_server = (Format-ServerTime $openTime)
            time = (To-Epoch $openTime)
        }
    )

    $deals = @(
        @{
            deal_ticket = 900001; position_ticket = 500001; symbol = "EURUSD"
            type = 0; entry = 0; volume = 0.10; price = 1.08500
            profit = 0.0; commission = -0.35; swap = 0.0
            magic = 0; comment = "sim-in"; time_server = (Format-ServerTime $yesterday.Date.AddHours(10))
            time = (To-Epoch $yesterday.Date.AddHours(10))
        },
        @{
            deal_ticket = 900002; position_ticket = 500001; symbol = "EURUSD"
            type = 0; entry = 1; volume = 0.10; price = 1.08700
            profit = 20.0; commission = -0.35; swap = 0.0
            magic = 0; comment = "sim-out"; time_server = (Format-ServerTime $yesterday.Date.AddHours(12).AddMinutes(30))
            time = (To-Epoch $yesterday.Date.AddHours(12).AddMinutes(30))
        },
        @{
            deal_ticket = 900003; position_ticket = 500002; symbol = "XAUUSD"
            type = 1; entry = 0; volume = 0.05; price = 2400.00
            profit = 0.0; commission = -0.25; swap = 0.0
            magic = 0; comment = "sim-in"; time_server = (Format-ServerTime $yesterday.Date.AddHours(14))
            time = (To-Epoch $yesterday.Date.AddHours(14))
        },
        @{
            deal_ticket = 900004; position_ticket = 500002; symbol = "XAUUSD"
            type = 1; entry = 1; volume = 0.05; price = 2390.00
            profit = 50.0; commission = -0.25; swap = 0.0
            magic = 0; comment = "sim-out"; time_server = (Format-ServerTime $yesterday.Date.AddHours(15))
            time = (To-Epoch $yesterday.Date.AddHours(15))
        }
    )

    return @{
        login = $Login
        server = $Server
        broker_utc_offset_seconds = $OffsetSeconds
        symbols = $symbols
        positions = $positions
        deals = $deals
        server_time = (Format-IsoTime $now)
    }
}

function Send-Sync {
    $payload = Build-SyncPayload
    $response = Invoke-Bridge -Path "/api/v1/mt5/sync" -Payload $payload
    if ($null -ne $response) {
        Write-Host ("[sync] " + (Get-ServerNow).ToString("HH:mm:ss") + " posiciones=" + $payload.positions.Count + " deals=" + $payload.deals.Count)
    }
}

function Build-SeriesPayload {
    param($CommandPayload)

    $now = Get-ServerNow
    $symbol = [string]$CommandPayload.symbol
    if ([string]::IsNullOrWhiteSpace($symbol)) {
        $symbol = "EURUSD"
    }
    $timeframe = [string]$CommandPayload.timeframe
    if ([string]::IsNullOrWhiteSpace($timeframe)) {
        $timeframe = "M15"
    }

    $seconds = switch ($timeframe.ToUpperInvariant()) {
        "M1" { 60 }
        "M5" { 300 }
        "M15" { 900 }
        "M30" { 1800 }
        "H1" { 3600 }
        "H4" { 14400 }
        "D1" { 86400 }
        "W1" { 604800 }
        default { 900 }
    }

    $fromValue = [long]$CommandPayload.from_time
    $toValue = [long]$CommandPayload.to_time
    $from = if ($fromValue -gt 0) { From-Epoch $fromValue } else { $now.AddHours(-4) }
    $to = if ($toValue -gt 0) { From-Epoch $toValue } else { $now }
    if ($to -gt $now) { $to = $now }
    if ($to -le $from) { $to = $from.AddSeconds($seconds * 40) }

    $digits = if ($symbol -eq "XAUUSD") { 2 } else { 5 }
    $price = if ($symbol -eq "XAUUSD") { 2395.0 } elseif ($symbol -eq "GBPUSD") { 1.2700 } else { 1.0860 }
    $drift = if ($symbol -eq "XAUUSD") { 0.02 } else { 0.00002 }

    $bars = @()
    $index = 0
    $time = $from
    while ($time -le $to -and $bars.Count -lt 800) {
        $open = $price
        $close = $open + $drift + ([math]::Sin($index / 4.0) * $drift * 3.0)
        $high = [math]::Max($open, $close) + $drift
        $low = [math]::Min($open, $close) - $drift
        $bars += @{
            time = (To-Epoch $time)
            open = [math]::Round($open, $digits)
            high = [math]::Round($high, $digits)
            low = [math]::Round($low, $digits)
            close = [math]::Round($close, $digits)
            volume = (Get-Random -Minimum 100 -Maximum 900)
            spread = 10
        }
        $price = $close
        $index++
        $time = $time.AddSeconds($seconds)
    }

    return @{
        login = $Login
        server = $Server
        trade_ticket = [long]$CommandPayload.trade_ticket
        symbol = $symbol
        timeframe = $timeframe
        from_time = (To-Epoch $from)
        to_time = (To-Epoch $to)
        bars = $bars
    }
}

function Send-Series {
    param($CommandPayload)

    $payload = Build-SeriesPayload -CommandPayload $CommandPayload
    $response = Invoke-Bridge -Path "/api/v1/mt5/series" -Payload $payload
    if ($null -ne $response) {
        Write-Host ("[series] ticket=" + $payload.trade_ticket + " " + $payload.symbol + " " + $payload.timeframe + " velas=" + $payload.bars.Count)
    }
}

function Send-CommandResult {
    param(
        [string]$CommandId,
        [bool]$Ok,
        [string]$ErrorText
    )

    $result = @{ command_id = $CommandId; ok = $Ok }
    if (-not $Ok -and -not [string]::IsNullOrWhiteSpace($ErrorText)) {
        $result.error = $ErrorText
    }
    Invoke-Bridge -Path "/api/v1/mt5/commands/result" -Payload $result | Out-Null
}

function Process-Commands {
    param($Response)

    if ($null -eq $Response -or $null -eq $Response.commands) {
        return
    }

    foreach ($command in @($Response.commands)) {
        if ($null -eq $command) {
            continue
        }

        $id = [string]$command.id
        if ([string]::IsNullOrWhiteSpace($id)) {
            continue
        }

        $ok = $true
        $errorText = $null
        try {
            switch ([string]$command.kind) {
                "sync" {
                    Send-Sync
                }
                "series" {
                    Send-Series -CommandPayload $command.payload
                }
                default {
                    $ok = $false
                    $errorText = "Comando no soportado: " + $command.kind
                }
            }
        }
        catch {
            $ok = $false
            $errorText = $_.Exception.Message
        }

        Send-CommandResult -CommandId $id -Ok $ok -ErrorText $errorText
        Write-Host ("[command] " + $command.kind + " ok=" + $ok)
    }
}

Write-Host "Plan de Trading - simulador MT5"
Write-Host ("Endpoint: " + $Base + " · cuenta " + $Login + " / " + $Server)
Write-Host "Detén con Ctrl+C."

Send-Sync
$lastSync = Get-Date

while ($true) {
    $heartbeat = Invoke-Bridge -Path "/api/v1/mt5/heartbeat" -Payload (Build-HeartbeatPayload)
    if ($null -ne $heartbeat) {
        Process-Commands -Response $heartbeat
    }

    if (((Get-Date) - $lastSync).TotalSeconds -ge $SyncSeconds) {
        Send-Sync
        $lastSync = Get-Date
    }

    Start-Sleep -Seconds ([math]::Max(1, $HeartbeatSeconds))
}
