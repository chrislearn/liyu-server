# 演示商品图来源

- `p00.png`–`p32.png`：复制自同仓库 `OctoSense/apps/liyu/resources/products/` 的既有演示素材（2026-09-26），与 app 目录下标一一对应。作为所有商品的缩略图、卡片图及详情图回退；不能据此声称真实商家授权或实物一致。
- `p00-detail.png`：2026-09-26 使用 Codex 内置 imagegen 生成的原创商品场景照片，供 ID 0 商品详情展示。原始输出为 `C:/Users/chris/.codex/generated_images/01a0dd00-ec50-7613-875f-2159c35f4695/exec-baee39fc-dd49-47a9-8023-0abac25b9220.png`，复制进项目后由服务器使用。
- `p12-detail.png`：同日 imagegen 原创电视家居场景，ID 12。原始输出 `exec-10f23bd0-17e6-4ab9-9527-7d5c864a944b.png`（同上生成目录）。
- `p22-detail.png`：同日 imagegen 原创陶瓷餐具场景，ID 22。原始输出 `exec-7e8c660e-7155-470c-9e58-18bb627f9920.png`（同上生成目录）。
- `p04-detail.png`：同日 imagegen 原创双人电影券卡面场景，ID 4。原始输出 `exec-c82236b8-cb7a-4b27-ae15-c8d41a5ebbd0.png`（同上生成目录）。
- `p05-detail.png`：同日 imagegen 原创帆布托特包场景，ID 5。原始输出 `exec-5a5074f5-cacb-4cd4-8f5d-87f2b2746e5e.png`（同上生成目录）。
- `p08-detail.png`：同日 imagegen 原创盲盒潮玩场景，ID 8。原始输出 `exec-916df953-4182-4c1a-80bd-06e8fc3cf4b8.png`（同上生成目录）。
- `p11-detail.png`：同日 imagegen 原创向日葵花束场景，ID 11。原始输出 `exec-22270649-2612-4043-af5d-f87b0ae0beaa.png`（同上生成目录）。
- `p27-detail.png`：同日 imagegen 原创新生儿礼盒场景，ID 27。原始输出 `exec-84c60a53-cdd6-4c2d-b9e9-4c399c882b1d.png`（同上生成目录）。

原创提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A premium 24-piece instant specialty coffee gift box on a light oak breakfast table, with several small unbranded matte coffee capsules arranged beside it and a glass of iced coffee. Warm natural morning window light, photorealistic editorial product photography, square composition, restrained beige and coffee-brown palette, clear subject at center. No visible text, logos, trademarks, people, watermark or real brand packaging.

`p12-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A premium slim 55-inch 4K television displayed in a warm contemporary apartment living room, showing a serene mountain landscape, visible narrow bezel and simple stand; natural daylight, believable scale, tasteful neutral furnishings, editorial lifestyle photography, square crop centered on the television. No visible text, logos, trademarks, people, watermark or real brand packaging.

`p22-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A matte cream ceramic tableware gift set for two on a light oak dining table: bowls, small plates and two cups, eight coordinated pieces with subtle handmade glaze, with folded linen napkin and soft morning light from a side window. Premium but believable lifestyle product photography, square composition, warm neutral palette, crisp detail. No visible text, logos, trademarks, people or watermark.

`p04-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce detail image for a fictional digital cinema gift voucher. A premium unbranded pair of dark navy cinema admission gift cards in a simple ivory sleeve, placed beside a small bowl of popcorn and subtle red velvet theater seat texture. Photorealistic still-life photography with warm cinema lighting, square composition, the cards deliberately blank with no writing, numbers, barcodes, logos, trademarks, people or watermark. Clearly communicate a two-person movie night gift without imitating a real ticket issuer.

`p05-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A sturdy ivory canvas tote bag with simple long straps and no printed design, casually placed on a light wooden bench beside a folded magazine and a neutral scarf in a bright city apartment entryway. Realistic thick canvas weave, believable proportions, soft daylight, premium editorial lifestyle photography, square crop. No visible text, logos, trademarks, people, watermark or real brand packaging.

`p08-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A charming unbranded collectible toy blind box opened on a tidy desk, with one original small whimsical creature figurine emerging beside the plain pastel box, soft matte vinyl texture and a few sealed identical blank boxes behind it. Playful but premium photorealistic product photography, gentle window lighting, square composition. Invent an original character unlike any existing franchise. No visible text, logos, trademarks, people or watermark.

`p11-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A fresh bouquet of exactly three sunflowers with eucalyptus foliage wrapped in simple natural kraft paper, standing in a clear vase on a small table in a sunlit apartment, warm welcoming mood, believable florist arrangement, realistic petals and leaves, photorealistic editorial still life, square composition. No visible text, logos, trademarks, people or watermark.

`p27-detail.png` 提示词：

> Use case: product-mockup. Asset type: ecommerce product detail photograph for a fictional LiYu gift catalog. A newborn gift box containing nine coordinated soft ivory and pale sage cotton baby essentials: tiny wrap shirt, bib, small socks and folded cloths, thoughtfully arranged in an open natural cardboard presentation box on a light linen blanket. No baby present. Premium but believable product photography, soft diffuse daylight, clearly visible cotton textures, square composition. No visible text, logos, trademarks, people or watermark.

图片为演示占位，不应冒充实际商品或品牌官方图片。商品文本中的现实品牌为前端既有演示数据，图片生成时明确避免真实商标。

八张原创详情图同时复制到 `OctoSense/apps/liyu/resources/products/`，覆盖 ID 0、4、5、8、11、12、22、27 对应的 `pNN.png`，让离线演示也展示相同的真实感商品图。原生成图和服务器 `pNN-detail.png` 保留；服务器 `pNN.png` 仍是旧版插画，可供其他商品风格回退。app 使用 1254×1254 PNG，资源按固定文件名编译，缩放由 `ImageFit.Stretch` 处理。
