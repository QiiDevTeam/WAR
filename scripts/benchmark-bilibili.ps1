[CmdletBinding()]
param(
    [string]$WarPath = (Join-Path $PSScriptRoot '..\target\release\war.exe'),
    [string]$ChromePath = 'C:\Program Files\Google\Chrome\Application\chrome.exe',
    [int]$LoadTimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
$utf8 = [System.Text.UTF8Encoding]::new($false)
$script:requestId = 0
$script:requestBytes = 0L
$script:responseBytes = 0L
$script:roundTrips = 0
$war = $null
$chrome = $null
$windowProcessId = $null
$stopwatch = $null

function Invoke-War {
    param(
        [Parameter(Mandatory)] [string]$Method,
        [Parameter(Mandatory)] [hashtable]$Params
    )

    $script:requestId++
    $request = @{
        id = $script:requestId
        method = $Method
        params = $Params
    } | ConvertTo-Json -Depth 100 -Compress
    if ($war.HasExited) {
        $stderr = $war.StandardError.ReadToEnd()
        throw "WAR exited before $Method (code $($war.ExitCode)). $stderr"
    }
    $script:requestBytes += $utf8.GetByteCount($request + $war.StandardInput.NewLine)
    $script:roundTrips++
    try {
        $war.StandardInput.WriteLine($request)
        $war.StandardInput.Flush()
    }
    catch {
        if ($war.HasExited) {
            $stderr = $war.StandardError.ReadToEnd()
            throw "WAR exited while sending $Method (code $($war.ExitCode)). $stderr"
        }
        throw
    }

    $line = $war.StandardOutput.ReadLine()
    if ($null -eq $line) {
        [void]$war.WaitForExit(250)
        $stderr = $war.StandardError.ReadToEnd()
        $exit = if ($war.HasExited) { $war.ExitCode } else { 'still-running' }
        throw "WAR closed stdout after request $($script:requestId) ($Method), exit=$exit. $stderr"
    }
    $script:responseBytes += $utf8.GetByteCount($line + "`n")
    $response = $line | ConvertFrom-Json -Depth 100
    if ($null -ne $response.error) {
        throw "WAR $Method failed: $($response.error | ConvertTo-Json -Depth 20 -Compress)"
    }
    return $response.result
}

function Get-AddressBar {
    param([hashtable]$Scope)
    return Invoke-War -Method 'inspect' -Params @{
        scope = $Scope
        name = '地址和搜索栏'
        fields = @('value', 'capabilities')
    }
}

try {
    if (-not (Test-Path -LiteralPath $WarPath)) {
        throw "WAR binary not found: $WarPath"
    }
    if (-not (Test-Path -LiteralPath $ChromePath)) {
        throw "Chrome binary not found: $ChromePath"
    }

    $profile = Join-Path (Split-Path $WarPath -Parent) ("war-bilibili-profile-{0}" -f [guid]::NewGuid().ToString('N'))
    $chrome = Start-Process -FilePath $ChromePath -ArgumentList @(
        "--user-data-dir=$profile",
        '--new-window',
        '--no-first-run',
        '--disable-background-mode',
        'about:blank'
    ) -WindowStyle Normal -PassThru

    $window = $null
    $windowDeadline = [datetime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 100
        $window = Get-Process chrome -ErrorAction SilentlyContinue |
            Where-Object {
                $_.StartTime -ge $chrome.StartTime.AddSeconds(-1) -and
                $_.MainWindowHandle -ne 0
            } |
            Select-Object -First 1
    } while ($null -eq $window -and [datetime]::UtcNow -lt $windowDeadline)
    if ($null -eq $window) {
        throw 'Chrome did not expose a visible top-level window within 10 seconds'
    }

    $windowProcessId = $window.Id
    $scope = @{ kind = 'window'; value = [uint64]$window.MainWindowHandle.ToInt64() }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Resolve-Path -LiteralPath $WarPath).Path
    $startInfo.Arguments = 'serve'
    $startInfo.WorkingDirectory = (Split-Path $WarPath -Parent)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardInputEncoding = $utf8
    $startInfo.StandardOutputEncoding = $utf8
    $startInfo.StandardErrorEncoding = $utf8
    $war = [System.Diagnostics.Process]::Start($startInfo)

    $homeUrl = 'https://www.bilibili.com/'
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

    $address = Get-AddressBar -Scope $scope
    $addressRef = "@$($address.node.id)"
    [void](Invoke-War -Method 'act' -Params @{
        expected_session_id = [string]$address.session_id
        expected_epoch = [uint64]$address.epoch
        actions = @(@{
            set_value = @{ target = $addressRef; value = $homeUrl }
        })
        postcondition = @{
            type = 'value_equals'; target = $addressRef; value = $homeUrl
        }
        stop_on_error = $true
        timeout_ms = 5000
        format = 'summary'
    })
    [void](Invoke-War -Method 'act' -Params @{
        actions = @(@{
            key = @{
                key = 'enter'
                modifiers = @{ ctrl = $false; alt = $false; shift = $false; meta = $false }
            }
        })
        stop_on_error = $true
        timeout_ms = 5000
        format = 'summary'
    })

    $homeQuery = Invoke-War -Method 'wait' -Params @{
        scope = $scope
        role = 'link'
        value_contains = 'bilibili.com/video/'
        required_capabilities = 'INVOKE'
        enabled = $true
        limit = 10
        fields = @('value', 'capabilities')
        timeout_ms = $LoadTimeoutSeconds * 1000
        poll_interval_ms = 250
    }
    $candidate = @($homeQuery.matches | Where-Object { ([string]$_.name).Length -ge 10 }) |
        Sort-Object -Property @(
            @{ Expression = { if ($_.name -match '\d{1,2}:\d{2}') { 0 } else { 1 } } },
            @{ Expression = { $_.id } }
        ) |
        Select-Object -First 1
    if ($null -eq $candidate) {
        throw 'No invokable Bilibili video candidate survived semantic filtering'
    }

    $candidateRef = "@$($candidate.id)"
    [void](Invoke-War -Method 'act' -Params @{
        expected_session_id = [string]$homeQuery.session_id
        expected_epoch = [uint64]$homeQuery.epoch
        actions = @(@{ invoke = $candidateRef })
        stop_on_error = $true
        timeout_ms = 5000
        format = 'summary'
    })

    $cleanTitle = ([string]$candidate.name -replace '\s+\d+(?:\.\d+)?万.*$', '').Trim()
    $expectedTitlePrefix = $cleanTitle.Substring(0, [math]::Min(8, $cleanTitle.Length))
    $addressQuery = Invoke-War -Method 'wait' -Params @{
        scope = $scope
        role = 'text_input'
        name = '地址和搜索栏'
        value_contains = 'bilibili.com/video/'
        limit = 1
        fields = @('value')
        timeout_ms = $LoadTimeoutSeconds * 1000
        poll_interval_ms = 250
    }
    $windowQuery = Invoke-War -Method 'wait' -Params @{
        scope = $scope
        role = 'window'
        name_contains = $expectedTitlePrefix
        limit = 1
        timeout_ms = $LoadTimeoutSeconds * 1000
        poll_interval_ms = 250
    }
    $stopwatch.Stop()

    $finalAddress = $addressQuery.matches[0]
    $finalWindow = $windowQuery.matches[0]

    $totalBytes = $script:requestBytes + $script:responseBytes
    [pscustomobject]@{
        status = 'verified'
        selected_title = [string]$candidate.name
        final_url = [string]$finalAddress.value
        final_window_title = [string]$finalWindow.name
        home_wait_ms = [uint64]$homeQuery.elapsed_ms
        home_observations = [uint64]$homeQuery.observations
        address_wait_ms = [uint64]$addressQuery.elapsed_ms
        address_observations = [uint64]$addressQuery.observations
        title_wait_ms = [uint64]$windowQuery.elapsed_ms
        title_observations = [uint64]$windowQuery.observations
        elapsed_ms = $stopwatch.ElapsedMilliseconds
        war_round_trips = $script:roundTrips
        request_bytes = $script:requestBytes
        response_bytes = $script:responseBytes
        total_bytes = $totalBytes
        estimated_tokens_utf8_div4 = [math]::Ceiling($totalBytes / 4.0)
        token_metric = 'estimate: ceil(total WAR JSONL UTF-8 bytes / 4)'
    } | ConvertTo-Json -Depth 10
}
finally {
    if ($null -ne $stopwatch -and $stopwatch.IsRunning) {
        $stopwatch.Stop()
    }
    if ($null -ne $war -and -not $war.HasExited) {
        try { $war.StandardInput.Close() } catch {}
        if (-not $war.WaitForExit(2000)) {
            $war.Kill($true)
        }
    }
    if ($null -ne $windowProcessId) {
        Stop-Process -Id $windowProcessId -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $chrome -and -not $chrome.HasExited) {
        Stop-Process -Id $chrome.Id -Force -ErrorAction SilentlyContinue
    }
}
