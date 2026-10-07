#!/usr/bin/env bash
set -e
cat > index.html <<'HTML'
<!doctype html>
<html>
<head><title>Checkout</title></head>
<body>
  <img src="logo.png">
  <div class="button" onclick="pay()">Pay now</div>
  <input type="email" placeholder="Email">
  <p style="color:#bbb;background:#fff">Card details are stored securely.</p>
</body>
</html>
HTML
