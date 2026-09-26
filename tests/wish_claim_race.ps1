# Run against a disposable local test database after starting the server.
# Two confirmed friends pay for the same open wish at once; exactly one may claim it.
$ErrorActionPreference = 'Stop'
$base = if ($env:LIYU_API_URL) { $env:LIYU_API_URL.TrimEnd('/') } else { 'http://127.0.0.1:8787' }

function Login($identifier) {
    $body = @{ identifier = $identifier; password = '123456' } | ConvertTo-Json
    (Invoke-RestMethod "$base/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body $body).token
}

function AuthHeaders($token) { @{ Authorization = "Bearer $token" } }

$ownerToken = Login 'demo@liyu.test'
$firstToken = Login 'linzhou@liyu.test'
$secondToken = Login 'chenxiao@liyu.test'
$owner = Invoke-RestMethod "$base/api/v1/me" -Headers (AuthHeaders $ownerToken)
$tomorrow = (Get-Date).AddDays(1).ToString('yyyy-MM-dd')
$wishBody = @{
    title = '并发认领测试'
    note = '两位好友同时送同一件'
    occasion = 'birthday'
    event_on = $tomorrow
    items = @(@{ product_id = 0; kind = '咖啡'; max_price_cents = 0; wants = '' })
} | ConvertTo-Json -Depth 5
$wishlist = Invoke-RestMethod "$base/api/v1/wishlists" -Method Post -Headers (AuthHeaders $ownerToken) -ContentType 'application/json' -Body $wishBody
$detail = Invoke-RestMethod "$base/api/v1/wishlists/$($wishlist.id)" -Headers (AuthHeaders $ownerToken)
$wishItemId = $detail.items[0].id

function MakeOrder($token) {
    Invoke-RestMethod "$base/api/v1/cart" -Method Delete -Headers (AuthHeaders $token) | Out-Null
    $cartBody = @{ product_id = 0; recipient_id = $owner.id; wish_item_id = $wishItemId } | ConvertTo-Json
    Invoke-RestMethod "$base/api/v1/cart/items" -Method Post -Headers (AuthHeaders $token) -ContentType 'application/json' -Body $cartBody | Out-Null
    $headers = AuthHeaders $token
    $headers['Idempotency-Key'] = [guid]::NewGuid().ToString()
    (Invoke-RestMethod "$base/api/v1/orders" -Method Post -Headers $headers -ContentType 'application/json' -Body '{}').id
}

$firstOrder = MakeOrder $firstToken
$secondOrder = MakeOrder $secondToken
$jobs = @(
    Start-Job -ScriptBlock {
        param($base, $id, $token)
        try {
            Invoke-RestMethod "$base/api/v1/orders/$id/pay-test" -Method Post -Headers @{ Authorization = "Bearer $token" } -ContentType 'application/json' -Body '{}' | Out-Null
            200
        } catch { [int]$_.Exception.Response.StatusCode }
    } -ArgumentList $base, $firstOrder, $firstToken
    Start-Job -ScriptBlock {
        param($base, $id, $token)
        try {
            Invoke-RestMethod "$base/api/v1/orders/$id/pay-test" -Method Post -Headers @{ Authorization = "Bearer $token" } -ContentType 'application/json' -Body '{}' | Out-Null
            200
        } catch { [int]$_.Exception.Response.StatusCode }
    } -ArgumentList $base, $secondOrder, $secondToken
)
$jobs | Wait-Job | Out-Null
$statuses = @($jobs | ForEach-Object { Receive-Job $_ })
$jobs | Remove-Job
if (@($statuses | Where-Object { $_ -eq 200 }).Count -ne 1 -or
    @($statuses | Where-Object { $_ -eq 409 }).Count -ne 1) {
    throw "Expected one paid order and one conflict, got $($statuses -join ', ')"
}
$ownerDetail = Invoke-RestMethod "$base/api/v1/wishlists/$($wishlist.id)" -Headers (AuthHeaders $ownerToken)
if ($ownerDetail.items[0].status -ne 'claimed') { throw 'Owner must see claimed only' }
$firstDetail = Invoke-RestMethod "$base/api/v1/wishlists/$($wishlist.id)" -Headers (AuthHeaders $firstToken)
$secondDetail = Invoke-RestMethod "$base/api/v1/wishlists/$($wishlist.id)" -Headers (AuthHeaders $secondToken)
$friendStatuses = @($firstDetail.items[0].status, $secondDetail.items[0].status)
if (@($friendStatuses | Where-Object { $_ -eq 'by_me' }).Count -ne 1 -or
    @($friendStatuses | Where-Object { $_ -eq 'claimed' }).Count -ne 1) {
    throw "Friend projections wrong: $($friendStatuses -join ', ')"
}
Write-Output "Concurrent claim passed: one paid, one 409; owner and friends see only role-safe status."
