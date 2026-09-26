param([string]$BaseUrl = 'http://127.0.0.1:8787')

$ErrorActionPreference = 'Stop'
function Login([string]$Identifier) {
    $body = @{ identifier = $Identifier; password = '123456' } | ConvertTo-Json -Compress
    $session = Invoke-RestMethod "$BaseUrl/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body $body
    return @{ Authorization = "Bearer $($session.token)" }
}
function ExpectStatus([scriptblock]$Action, [int]$Expected) {
    try {
        & $Action | Out-Null
        throw "Expected HTTP $Expected but request succeeded"
    } catch {
        if (-not $_.Exception.Response -or [int]$_.Exception.Response.StatusCode -ne $Expected) { throw }
    }
}

$sender = Login 'demo@liyu.test'
$recipient = Login 'linzhou@liyu.test'
$stranger = Login 'chenxiao@liyu.test'
$products = Invoke-RestMethod "$BaseUrl/api/v1/catalog?limit=50"
if ($products.items.Count -ne 33) { throw "Expected 33 catalog products" }

$delivery = Invoke-RestMethod "$BaseUrl/api/v1/gifts/900001/delivery-summary" -Headers $sender
$fields = @($delivery.PSObject.Properties.Name | Sort-Object)
if (($fields -join ',') -ne 'carrier_delivered,gift_id,recipient_confirmed') { throw "Sender delivery projection leaked fields: $fields" }
$shipment = Invoke-RestMethod "$BaseUrl/api/v1/shipments/900001" -Headers $recipient
if (-not $shipment.tracking_number -or -not $shipment.recipient.address) { throw 'Recipient shipment is incomplete' }
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/shipments/900001" -Headers $sender } 404
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/shipments/900001" -Headers $stranger } 404
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/gifts/900001/delivery-summary" -Headers $recipient } 404
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/state" -Headers $sender } 410

$recipientProfile = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $recipient
$senderProfile = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $sender
if ($recipientProfile.id -eq $senderProfile.id) { throw 'Profiles are not account-scoped' }

$cartBody = @{ product_id = 0; recipient_id = $recipientProfile.id } | ConvertTo-Json -Compress
$cartItem = Invoke-RestMethod "$BaseUrl/api/v1/cart/items" -Method Post -Headers $sender -ContentType 'application/json' -Body $cartBody
$quote = Invoke-RestMethod "$BaseUrl/api/v1/orders/quote" -Method Post -Headers $sender
if ($quote.total_cents -ne 10900) { throw 'Server quote used the wrong price' }
$sender['Idempotency-Key'] = "e2e-$([guid]::NewGuid().ToString('N'))"
$order = Invoke-RestMethod "$BaseUrl/api/v1/orders" -Method Post -Headers $sender
$again = Invoke-RestMethod "$BaseUrl/api/v1/orders" -Method Post -Headers $sender
if ($again.id -ne $order.id) { throw 'Order idempotency failed' }
$paid = Invoke-RestMethod "$BaseUrl/api/v1/orders/$($order.id)/pay-test" -Method Post -Headers $sender
$paidAgain = Invoke-RestMethod "$BaseUrl/api/v1/orders/$($order.id)/pay-test" -Method Post -Headers $sender
if ($paid.status -ne 'paid_test' -or $paidAgain.id -ne $paid.id) { throw 'Test payment was not idempotent' }
$detail = Invoke-RestMethod "$BaseUrl/api/v1/orders/$($order.id)" -Headers $sender
if ($detail.items.Count -ne 1 -or -not $detail.items[0].gift_id) { throw 'Payment did not create a gift' }
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/orders/$($order.id)" -Headers $stranger } 404

Write-Output "LiYu end-to-end API checks passed: 33 products, scoped profile/parcel/order, cart, idempotent payment, legacy state disabled."
