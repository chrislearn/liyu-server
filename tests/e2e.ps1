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

# --- Avatar upload API: negative and positive end-to-end checks ---
# Build a real 24x16 PNG in memory (valid CRC via System.Drawing is not
# available everywhere, so hand-roll a minimal true PNG with zlib store blocks).
function New-TestPng([int]$Width, [int]$Height) {
    Add-Type -AssemblyName System.IO.Compression
    $ms = [System.IO.MemoryStream]::new()
    $bw = [System.IO.BinaryWriter]::new($ms)
    $bw.Write([byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
    function Write-Chunk([System.IO.BinaryWriter]$w, [string]$Type, [byte[]]$Data) {
        $len = [BitConverter]::GetBytes([uint32]$Data.Length); [Array]::Reverse($len)
        $w.Write($len); $w.Write([Text.Encoding]::ASCII.GetBytes($Type)); $w.Write($Data)
        $crcInput = [Text.Encoding]::ASCII.GetBytes($Type) + $Data
        # CRC-32 (PNG polynomial), kept in uint32 to avoid sign issues.
        $crc = [uint32]0xFFFFFFFF
        foreach ($b in $crcInput) {
            $crc = $crc -bxor [uint32]$b
            for ($i = 0; $i -lt 8; $i++) {
                $mask = [uint32](0 - ($crc -band 1))
                $crc = ($crc -shr 1) -bxor ([uint32]0xEDB88320 -band $mask)
            }
        }
        $crcBytes = [BitConverter]::GetBytes([uint32]($crc -bxor [uint32]0xFFFFFFFF)); [Array]::Reverse($crcBytes)
        $w.Write($crcBytes)
    }
    $ihdrBytes = [byte[]]::new(13)
    $wBytes = [BitConverter]::GetBytes([uint32]$Width); [Array]::Reverse($wBytes)
    $hBytes = [BitConverter]::GetBytes([uint32]$Height); [Array]::Reverse($hBytes)
    [Array]::Copy($wBytes, 0, $ihdrBytes, 0, 4)
    [Array]::Copy($hBytes, 0, $ihdrBytes, 4, 4)
    $ihdrBytes[8] = 8; $ihdrBytes[9] = 2 # 8-bit RGB
    Write-Chunk $bw 'IHDR' $ihdrBytes
    # Raw scanlines: filter byte 0 + RGB pixels, zlib-stored (no compression).
    $raw = [byte[]]::new($Height * (1 + 3 * $Width))
    for ($y = 0; $y -lt $Height; $y++) { $raw[$y * (1 + 3 * $Width)] = 0 }
    $zms = [System.IO.MemoryStream]::new()
    $zw = [System.IO.BinaryWriter]::new($zms)
    $zw.Write([byte]0x78); $zw.Write([byte]0x01) # zlib header, no compression
    $pos = 0
    while ($pos -lt $raw.Length) {
        $blockLen = [Math]::Min(65535, $raw.Length - $pos)
        $final = if ($pos + $blockLen -eq $raw.Length) { [byte]1 } else { [byte]0 }
        $zw.Write($final)
        $zw.Write([BitConverter]::GetBytes([uint16]$blockLen))
        $zw.Write([BitConverter]::GetBytes([uint16](-bnot $blockLen -band 0xFFFF)))
        $zw.Write($raw, $pos, $blockLen)
        $pos += $blockLen
    }
    # Adler-32 of raw
    $a = 1; $b = 0
    foreach ($byte in $raw) { $a = ($a + $byte) % 65521; $b = ($b + $a) % 65521 }
    $adler = [BitConverter]::GetBytes([uint32](($b -shl 16) -bor $a)); [Array]::Reverse($adler)
    $zw.Write($adler)
    Write-Chunk $bw 'IDAT' $zms.ToArray()
    Write-Chunk $bw 'IEND' ([byte[]]::new(0))
    $bw.Flush()
    return $ms.ToArray()
}

$png24 = New-TestPng 24 16
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
$bombPng = New-TestPng 64 64
$bombPng[16] = 0xFF; $bombPng[17] = 0xFF; $bombPng[18] = 0xFF; $bombPng[19] = 0xFF
$bombPng[20] = 0xFF; $bombPng[21] = 0xFF; $bombPng[22] = 0xFF; $bombPng[23] = 0xFF
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $bombPng } 422
# 6. Oversized body (> 1 MiB) is rejected.
$big = [byte[]]::new(1048577)
[Array]::Copy($png24, $big, $png24.Length)
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $big } 413
# 7. Valid upload succeeds and returns the media URL + dimensions.
$upload = Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Post -Headers $sender -ContentType 'image/png' -Body $png24
if ($upload.width -ne 24 -or $upload.height -ne 16 -or $upload.content_type -ne 'image/png') { throw 'Avatar upload returned wrong metadata' }
if ($upload.avatar_url -notmatch '^/api/v1/media/avatars/[0-9a-f]{32}$') { throw "Unexpected avatar_url: $($upload.avatar_url)" }
# 8. The media endpoint serves the stored bytes with the right content type.
$media = Invoke-WebRequest "$BaseUrl$($upload.avatar_url)"
if ($media.StatusCode -ne 200 -or $media.Headers['Content-Type'] -ne 'image/png') { throw 'Avatar media fetch failed' }
if ($media.Content.Length -ne $png24.Length) { throw 'Avatar media bytes differ from upload' }
# 9. Profile reflects the uploaded avatar URL.
$senderAfter = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $sender
if ($senderAfter.avatar_url -ne $upload.avatar_url) { throw 'Profile avatar_url was not updated' }
# 10. Deleting the avatar clears the profile pointer; media then 404s.
Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Delete -Headers $sender | Out-Null
$senderCleared = Invoke-RestMethod "$BaseUrl/api/v1/me/profile" -Headers $sender
if ($null -ne $senderCleared.avatar_url) { throw 'Avatar delete did not clear profile avatar_url' }
ExpectStatus { Invoke-WebRequest "$BaseUrl$($upload.avatar_url)" } 404
# 11. Media endpoint hardens the id: mixed-case/braced UUID spellings 404.
$rawId = $upload.avatar_url -replace '^/api/v1/media/avatars/', ''
$upperId = $rawId.ToUpper()
ExpectStatus { Invoke-WebRequest "$BaseUrl/api/v1/media/avatars/$upperId" } 404
ExpectStatus { Invoke-WebRequest "$BaseUrl/api/v1/media/avatars/not-a-uuid" } 404
# 12. Another user cannot delete or overwrite via someone else's session.
ExpectStatus { Invoke-RestMethod "$BaseUrl/api/v1/me/avatar" -Method Delete } 401

Write-Output "LiYu end-to-end API checks passed: registration/session rotation, 33 products, scoped profile/parcel/order, cart, payment, puzzle, acceptance, legacy state disabled, and avatar upload/media/delete hardening."
