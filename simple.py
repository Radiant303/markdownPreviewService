# %% Python调用接口示例
import requests

url = "https://md.guoliguoli.cc/generate"

markdown = """这是对 **Markdown转PNG** 服务的测试。

## 功能

- 支持标题
- 支持列表
- 支持代码块
"""

response = requests.post(
    url,
    data=markdown.encode("utf-8"),
    headers={"Content-Type": "text/plain; charset=utf-8"},
    timeout=30,
)

response.raise_for_status()

with open("output.png", "wb") as f:
    f.write(response.content)

print("PNG 已保存到 output.png")
