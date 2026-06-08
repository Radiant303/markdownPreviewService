# %% Python调用接口示例
import requests

url = "http://localhost:3001/generate"

markdown = """# XU JP
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

# 


# P 
