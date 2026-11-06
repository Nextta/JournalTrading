param(
    [string]$ApiKey = $env:JOURNAL_TRADING_API_KEY,
    [int]$Port = 4477,
    [long]$Login = 1001,
    [string]$Server = "Simulator",
    [int]$OffsetSeconds = 0,
    [int]$HeartbeatSeconds = 5,
    [int]$SyncSeconds = 15
)

if ([string]::IsNullOrWhiteSpace($ApiKey)) {
    throw "Falta la API key. Usa -ApiKey <valor> o define JOURNAL_TRADING_API_KEY. La encuentras en Ajustes > API key."
}

$Base = "http://127.0.0.1:$Port"
$Headers = @{ "X-Api-Key" = $ApiKey }

function Get-ServerNow {
    return [DateTime]::UtcNow.AddSeconds($OffsetSeconds)
}

function Format-IsoTime([datetime]$Value) {
    return $Value.ToString("yyyy-MM-ddTHH:mm:ss")
}

function To-Epoch([datetime]$Value) {
    return [DateTimeOffset]::new($Value, [TimeSpan]::Zero).ToUnixTimeSeconds()
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
        display_name = "NT8 Simulator"
        company = "NinjaTrader 8 · Simulator"
        currency = "USD"
        leverage = $null
        broker_utc_offset_seconds = $OffsetSeconds
        balance = 50000.0
        equity = 50312.5
        margin = 12000.0
        free_margin = 38312.5
        ea_version = "nt8-sim-1.0.0"
        terminal_build = 8
        terminal_data_path = "C:\Simulator\NinjaTrader 8"
        server_time = (Format-IsoTime $now)
    }
}

function Build-SyncPayload {
    $now = Get-ServerNow
    $yesterday = $now.AddDays(-1)
    $openTime = $now.AddHours(-2)

    $symbols = @(
        @{
            symbol = "ES 12-25"; description = "E-mini S&P 500"; digits = 2
            tick_size = 0.25; tick_value = 12.5; contract_size = 1.0
            currency_profit = "USD"
        },
        @{
            symbol = "NQ 12-25"; description = "E-mini Nasdaq 100"; digits = 2
            tick_size = 0.25; tick_value = 5.0; contract_size = 1.0
            currency_profit = "USD"
        }
    )

    $positions = @(
        @{
            ticket = 700003; symbol = "ES 12-25"; type = 0; volume = 1.0
            price_open = 4510.00; sl = 4505.00; tp = 4520.00
            profit = 62.5; swap = 0.0; commission = 0.0
            magic = 0; comment = "open"; time_server = (Format-IsoTime $openTime)
            time = (To-Epoch $openTime); time_utc = (Format-IsoTime $openTime) + "Z"
            tick_size = 0.25; tick_value = 12.5
        }
    )

    $deals = @(
        @{
            deal_ticket = 800001; position_ticket = 700001; symbol = "ES 12-25"
            type = 0; entry = 0; volume = 1.0; price = 4500.00
            profit = 0.0; commission = -2.50; swap = 0.0; reason = -1
            magic = 0; comment = "Entry"; time_server = (Format-IsoTime $yesterday.Date.AddHours(10))
            time = (To-Epoch $yesterday.Date.AddHours(10)); time_utc = (Format-IsoTime $yesterday.Date.AddHours(10)) + "Z"
        },
        @{
            deal_ticket = 800002; position_ticket = 700001; symbol = "ES 12-25"
            type = 1; entry = 1; volume = 1.0; price = 4502.00
            profit = 100.0; commission = -2.50; swap = 0.0; reason = 5
            magic = 0; comment = "Profit target"; time_server = (Format-IsoTime $yesterday.Date.AddHours(11))
            time = (To-Epoch $yesterday.Date.AddHours(11)); time_utc = (Format-IsoTime $yesterday.Date.AddHours(11)) + "Z"
        },
        @{
            deal_ticket = 800003; position_ticket = 700002; symbol = "NQ 12-25"
            type = 1; entry = 0; volume = 1.0; price = 16000.00
            profit = 0.0; commission = -2.00; swap = 0.0; reason = -1
            magic = 0; comment = "Entry"; time_server = (Format-IsoTime $yesterday.Date.AddHours(14))
            time = (To-Epoch $yesterday.Date.AddHours(14)); time_utc = (Format-IsoTime $yesterday.Date.AddHours(14)) + "Z"
        },
        @{
            deal_ticket = 800004; position_ticket = 700002; symbol = "NQ 12-25"
            type = 0; entry = 1; volume = 1.0; price = 15990.00
            profit = 200.0; commission = -2.00; swap = 0.0; reason = 4
            magic = 0; comment = "Stop loss"; time_server = (Format-IsoTime $yesterday.Date.AddHours(15))
            time = (To-Epoch $yesterday.Date.AddHours(15)); time_utc = (Format-IsoTime $yesterday.Date.AddHours(15)) + "Z"
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
    $response = Invoke-Bridge -Path "/api/v1/nt8/sync" -Payload $payload
    if ($null -ne $response) {
        Write-Host ("[sync] " + (Get-ServerNow).ToString("HH:mm:ss") + " posiciones=" + $payload.positions.Count + " deals=" + $payload.deals.Count)
    }
}

function Build-SeriesPayload {
    param($CommandPayload)

    $now = Get-ServerNow
    $symbol = [string]$CommandPayload.symbol
    if ([string]::IsNullOrWhiteSpace($symbol)) {
        $symbol = "ES 12-25"
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
    $from = if ($fromValue -gt 0) { (From-Epoch $fromValue).AddSeconds(-$seconds * 20) } else { $now.AddHours(-4) }
    $to = if ($toValue -gt 0) { (From-Epoch $toValue).AddSeconds($seconds * 10) } else { $now }
    if ($to -gt $now) { $to = $now }
    if ($to -le $from) { $to = $from.AddSeconds($seconds * 40) }

    $isNq = $symbol -like "NQ*"
    $digits = 2
    $price = if ($isNq) { 15995.0 } else { 4505.0 }
    $drift = if ($isNq) { 0.5 } else { 0.25 }

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
            spread = 1
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
    $response = Invoke-Bridge -Path "/api/v1/nt8/series" -Payload $payload
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
    Invoke-Bridge -Path "/api/v1/nt8/commands/result" -Payload $result | Out-Null
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

Write-Host "JournalTrading - simulador NinjaTrader 8"
Write-Host ("Endpoint: " + $Base + " · cuenta " + $Login + " / " + $Server)
Write-Host "Detén con Ctrl+C."

Send-Sync
$lastSync = Get-Date

while ($true) {
    $heartbeat = Invoke-Bridge -Path "/api/v1/nt8/heartbeat" -Payload (Build-HeartbeatPayload)
    if ($null -ne $heartbeat) {
        Process-Commands -Response $heartbeat
    }

    if (((Get-Date) - $lastSync).TotalSeconds -ge $SyncSeconds) {
        Send-Sync
        $lastSync = Get-Date
    }

    Start-Sleep -Seconds ([math]::Max(1, $HeartbeatSeconds))
}
