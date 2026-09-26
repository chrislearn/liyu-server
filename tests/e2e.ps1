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
$newIdentifier = "e2e-$([guid]::NewGuid().ToString('N'))"
$registration = @{ identifier = $newIdentifier; display_name = '测试新用户'; password = '123456'; code = '123456' } | ConvertTo-Json -Compress
$newAccount = Invoke-RestMethod "$BaseUrl/api/v1/auth/register" -Method Post -ContentType 'application/json' -Body $registration
if (-not $newAccount.token) { throw 'Registration did not create a session' }
$oldSession = @{ Authorization = "Bearer $($newAccount.token)" }
$rotated = Invoke-RestMethod "$BaseUrl/api/v1/auth/refresh" -Method Post -Headers $oldSession
if (-not $rotated.token -or $rotated.token -eq $newAccount.token) { throw 'Session refresh did not rotate token' }
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me" -Headers $oldSession } 401
$newSession = @{ Authorization = "Bearer $($rotated.token)" }
Invoke-RestMethod "$BaseUrl/api/v1/auth/logout" -Method Post -Headers $newSession | Out-Null
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me" -Headers $newSession } 401
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/auth/register" -Method Post -ContentType 'application/json' -Body (@{ identifier = "bad-$newIdentifier"; password = '123456'; code = '000000' } | ConvertTo-Json -Compress) } 400
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body (@{ identifier = $newIdentifier; password = 'wrong' } | ConvertTo-Json -Compress) } 401
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
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/state" -Method Put -Headers $sender -ContentType 'application/json' -Body '{}' } 410

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

$giftId = $detail.items[0].gift_id
$puzzleBody = @{ unlock_kind = 'question'; clue = '测试答案是什么'; answer = '礼遇'; message = '演示祝福' } | ConvertTo-Json -Compress
Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/puzzle" -Method Put -Headers $sender -ContentType 'application/json' -Body $puzzleBody | Out-Null
$sealed = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $recipient
if ($sealed.state -ne 'sealed' -or $sealed.PSObject.Properties.Name -contains 'product_id' -or ($sealed | ConvertTo-Json -Compress) -match '礼遇|演示祝福') {
    throw 'Sealed gift leaked its answer, product, or private message'
}
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $stranger } 404
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/open" -Method Post -Headers $sender } 404
Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/open" -Method Post -Headers $recipient | Out-Null
$wrong = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/answer" -Method Post -Headers $recipient -ContentType 'application/json' -Body '{"answer":"错误"}'
if ($wrong.correct -or $wrong.attempts_left -ne 2) { throw 'Wrong answer was not counted' }
$right = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/answer" -Method Post -Headers $recipient -ContentType 'application/json' -Body '{"answer":"礼遇"}'
if (-not $right.correct -or $right.state -ne 'revealed') { throw 'Correct answer did not reveal gift' }
$acceptBody = @{ recipient_name = '林舟'; recipient_phone = '13900000000'; recipient_address = '演示路 8 号（测试）' } | ConvertTo-Json -Compress
Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/accept" -Method Post -Headers $recipient -ContentType 'application/json' -Body $acceptBody | Out-Null
$senderGift = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $sender
$recipientGift = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $recipient
if ($senderGift.state -ne 'handled' -or $senderGift.price_cents -ne 10900 -or $recipientGift.state -ne 'accepted') { throw 'Gift role projections are inconsistent' }
if (($senderGift | ConvertTo-Json -Compress) -match '13900000000|演示路|tracking|voucher|answer|contract|exchanged|cashed_out') { throw 'Sender gift projection leaked recipient-private fields' }
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/shipments/$giftId" -Headers $sender } 404
$acceptedShipment = Invoke-RestMethod "$BaseUrl/api/v1/shipments/$giftId" -Headers $recipient
if ($acceptedShipment.recipient.phone -ne '13900000000') { throw 'Recipient shipment missing address snapshot' }

Write-Output "LiYu end-to-end API checks passed: registration/session rotation, 33 products, scoped profile/parcel/order, cart, payment, puzzle, acceptance, and legacy state disabled."
