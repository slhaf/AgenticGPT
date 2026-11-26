# Screenshots
* Browser screenshots are JPEG: use `.jpg` and `image/jpeg`.
* If you take a screenshot that the user should see, return it as an image from `browser.repl`:
  ```js
  nodeRepl.emitImage(await tab.screenshot());
  ```
  `tab.screenshot()` returns JPEG bytes. The call above includes them as an image in the MCP tool result for the client to display.
* If the user has asked you to take screenshots, return the requested images and refer to them in your final response.
* If the user has asked you to test a website as part of development, you should take screenshots at key moments and return them through the same tool.
