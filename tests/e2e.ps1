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

$orderBody = @{ product_id = 0; recipient_id = $recipientProfile.id } | ConvertTo-Json -Compress
$quote = Invoke-RestMethod "$BaseUrl/api/v1/orders/quote" -Method Post -Headers $sender -ContentType 'application/json' -Body $orderBody
if ($quote.total_cents -ne 10900) { throw 'Server quote used the wrong price' }
$sender['Idempotency-Key'] = "e2e-$([guid]::NewGuid().ToString('N'))"
$order = Invoke-RestMethod "$BaseUrl/api/v1/orders" -Method Post -Headers $sender -ContentType 'application/json' -Body $orderBody
$again = Invoke-RestMethod "$BaseUrl/api/v1/orders" -Method Post -Headers $sender -ContentType 'application/json' -Body $orderBody
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
if ($sealed.state -ne 'sealed' -or $sealed.product_id -ne 0 -or -not $sealed.product.name -or -not $sealed.product.image_detail_url -or $sealed.sender -ne $null -or $sealed.clue -ne '' -or ($sealed | ConvertTo-Json -Compress) -match '演示祝福') {
    throw 'Sealed gift did not include its product or leaked sender, answer, or private message'
}
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $stranger } 404
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/open" -Method Post -Headers $sender } 404
Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/open" -Method Post -Headers $recipient | Out-Null
$opened = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $recipient
if ($opened.product_id -ne 0 -or $opened.sender -ne $null -or $opened.clue -ne '测试答案是什么') { throw 'Opened gift must show product and clue while hiding sender' }
$wrong = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/answer" -Method Post -Headers $recipient -ContentType 'application/json' -Body '{"answer":"错误"}'
if ($wrong.correct -or $wrong.attempts_left -ne 2) { throw 'Wrong answer was not counted' }
$right = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/answer" -Method Post -Headers $recipient -ContentType 'application/json' -Body '{"answer":"礼遇"}'
if (-not $right.correct -or $right.state -ne 'revealed') { throw 'Correct answer did not reveal gift' }
$acceptBody = @{ recipient_name = '林舟'; recipient_phone = '13900000000'; recipient_address = '演示路 8 号（测试）' } | ConvertTo-Json -Compress
Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId/accept" -Method Post -Headers $recipient -ContentType 'application/json' -Body $acceptBody | Out-Null
$senderGift = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $sender
$recipientGift = Invoke-RestMethod "$BaseUrl/api/v1/gifts/$giftId" -Headers $recipient
if ($senderGift.state -ne 'handled' -or $senderGift.price_cents -ne 10900 -or $recipientGift.state -ne 'accepted') { throw 'Gift role projections are inconsistent' }
if (($senderGift | ConvertTo-Json -Compress) -match '13900000000|演示路|tracking|voucher|answer|exchanged|cashed_out') { throw 'Sender gift projection leaked recipient-private fields' }
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/shipments/$giftId" -Headers $sender } 404
$acceptedShipment = Invoke-RestMethod "$BaseUrl/api/v1/shipments/$giftId" -Headers $recipient
if ($acceptedShipment.recipient.phone -ne '13900000000') { throw 'Recipient shipment missing address snapshot' }

# --- Avatar upload API: negative and positive end-to-end checks ---
# Upload sample: the repository's real 400x400 PNG (test-data/products/p23.png,
# ~15 KB). Hand-rolled in-memory PNGs proved brittle across PowerShell builds
# (uint32 casts, zlib framing), so the positive path uses bytes the real
# decoder is guaranteed to accept.
$png24 = [System.IO.File]::ReadAllBytes("$PSScriptRoot/../test-data/products/p23.png")
# 1. Unauthenticated upload is rejected.
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -ContentType 'image/png' -Body $png24 } 401
# 2. Declared content type outside the allow-list is rejected.
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/gif' -Body $png24 } 415
# 3. Declared type disagrees with the actual bytes (declared jpeg, body png).
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/jpeg' -Body $png24 } 422
# 4. Header-only fake (valid sniff, no decodable image data) is rejected.
$fakePng = [byte[]](0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A, 0,0,0,13) + [Text.Encoding]::ASCII.GetBytes('IHDR') + [byte[]](0,0,0,64, 0,0,0,32, 8,2,0,0,0) + [byte[]]::new(20) + [Text.Encoding]::ASCII.GetBytes('IEND') + [byte[]](0xAE,0x42,0x60,0x82)
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $fakePng } 422
# 5. Decompression bomb dimensions (declared 65535x65535) rejected pre-decode.
# Corrupt only the IHDR width/height bytes of the real PNG.
$bombPng = [byte[]]$png24.Clone()
$bombPng[16] = 0xFF; $bombPng[17] = 0xFF; $bombPng[18] = 0xFF; $bombPng[19] = 0xFF
$bombPng[20] = 0xFF; $bombPng[21] = 0xFF; $bombPng[22] = 0xFF; $bombPng[23] = 0xFF
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $bombPng } 422
# 6. Oversized body (> 1 MiB) is rejected with 413. Sent via curl:
# Invoke-RestMethod reports a broken pipe when the server rejects the request
# before the upload stream finishes, which hides the real status code.
$bigPath = Join-Path ([System.IO.Path]::GetTempPath()) "liyu-avatar-big-$([Guid]::NewGuid().ToString('N')).bin"
try {
    $big = [byte[]]::new(1048577)
    [Array]::Copy($png24, $big, $png24.Length)
    [System.IO.File]::WriteAllBytes($bigPath, $big)
    $token = $sender['Authorization'] -replace '^Bearer ', ''
    $curlOut = & curl -sS -o /dev/null -w '%{http_code}' -X POST "$BaseUrl/api/v1/me/avatar" -H "Authorization: Bearer $token" -H 'Content-Type: image/png' --data-binary "@$bigPath" 2>&1
    if ($LASTEXITCODE -ne 0 -and "$curlOut" -notmatch '^\d{3}$') { throw "curl oversized upload failed: $curlOut" }
    $code = ("$curlOut" -split "`n")[-1].Trim()
    if ($code -ne '413') { throw "Oversized avatar upload: expected 413, got $code" }
} finally {
    Remove-Item $bigPath -ErrorAction SilentlyContinue
}
# 7. Valid upload succeeds and returns the media URL + dimensions.
$upload = Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $png24
if ($upload.width -ne 400 -or $upload.height -ne 400 -or $upload.content_type -ne 'image/png') { throw 'Avatar upload returned wrong metadata' }
if ($upload.avatar_url -notmatch '^/api/v1/media/avatars/[0-9a-f]{32}$') { throw "Unexpected avatar_url: $($upload.avatar_url)" }
# 8. The media endpoint serves the stored bytes with the right content type.
$media = Invoke-WebRequest "$BaseUrl$($upload.avatar_url)"
if ($media.StatusCode -ne 200 -or $media.Headers['Content-Type'] -ne 'image/png') { throw 'Avatar media fetch failed' }
if ($media.Content.Length -ne $png24.Length) { throw 'Avatar media bytes differ from upload' }
# 9. Profile reflects the uploaded avatar URL.
$senderAfter = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $sender
if ($senderAfter.avatar_url -ne $upload.avatar_url) { throw 'Profile avatar_url was not updated' }
# 10. Deleting the avatar restores the default profile image; uploaded media 404s.
Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Delete -Headers $sender | Out-Null
$senderCleared = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $sender
if ($senderCleared.avatar_url -ne "/api/v1/media/default-avatars/$($senderCleared.id)") { throw 'Avatar delete did not restore the default avatar' }
ExpectStatus { Invoke-WebRequest "$BaseUrl$($upload.avatar_url)" } 404
# 11. Media endpoint hardens the id: mixed-case/braced UUID spellings 404.
$rawId = $upload.avatar_url -replace '^/api/v1/media/avatars/', ''
$upperId = $rawId.ToUpper()
ExpectStatus { Invoke-WebRequest "$BaseUrl/api/v1/media/avatars/$upperId" } 404
ExpectStatus { Invoke-WebRequest "$BaseUrl/api/v1/media/avatars/not-a-uuid" } 404
# 12. Another user cannot delete or overwrite via someone else's session.
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Delete } 401

Write-Output "LiYu end-to-end API checks passed: registration/session rotation, 33 products, scoped profile/parcel/direct order, payment, puzzle, acceptance, legacy state disabled, and avatar upload/media/delete hardening."
