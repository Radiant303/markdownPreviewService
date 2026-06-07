# Markdown Preview Service

一个基于 Rust + Axum 的 Markdown 转 PNG 图片服务。

服务接收 Markdown 原文，渲染成带卡片样式的 PNG 图片返回。当前样式是浅灰背景、白色圆角卡片、橙色点缀、深色代码块，并保留代码语法高亮。

## 功能特性

- Markdown 转 PNG
- 支持中文内容
- 支持标题、段落、列表、分割线
- 支持任务列表
- 支持代码块语法高亮
- 代码块不显示语言标签，例如 `rust` / `python`
- 长中文段落自动换行
- 默认使用系统字体 `0xProto Nerd Font Mono`，不存在时自动降级

## 快速开始

### 方式一：直接运行 release exe

如果已经编译好了，可以直接运行：

```powershell
D:\GitHub\markdownPreviewService\target\release\markdown-preview-service.exe
```

或者在项目目录中运行：

```powershell
.\target\release\markdown-preview-service.exe
```

启动成功后会看到类似输出：

```text
Server listening on 0.0.0.0:3001
```

浏览器访问：

```text
http://localhost:3001/
```

如果看到下面内容，说明服务已启动：

```text
Markdown-to-PNG Service is running
```

### 方式二：使用 Cargo 运行

```powershell
cargo run
```

### 方式三：重新编译 release 版本

修改代码后，需要重新编译 release exe：

```powershell
cargo build --release
```

然后再运行：

```powershell
.\target\release\markdown-preview-service.exe
```

## 修改端口

默认端口是 `3001`。

可以通过环境变量 `PORT` 修改端口。

PowerShell 示例：

```powershell
$env:PORT=8080
.\target\release\markdown-preview-service.exe
```

然后访问：

```text
http://localhost:8080/
```

## API 说明

### 健康检查

```http
GET /
```

响应：

```text
Markdown-to-PNG Service is running
```

### 生成 PNG

```http
POST /generate
```

请求体直接传 Markdown 原文，不是 JSON。

推荐请求头：

```http
Content-Type: text/plain; charset=utf-8
```

响应内容：

```http
Content-Type: image/png
```

## Apifox 调用方式

1. 启动服务

   ```powershell
   .\target\release\markdown-preview-service.exe
   ```

2. 在 Apifox 新建请求

   - Method：`POST`
   - URL：`http://localhost:3001/generate`

3. 设置 Headers

   | Key | Value |
   | --- | --- |
   | `Content-Type` | `text/plain; charset=utf-8` |

4. 设置 Body

   Body 选择 `raw`，类型选择 `Text`，然后填写 Markdown：

   ````markdown
   # 测试标题

   这是一段 Markdown 内容。

   ## 列表

   - 第一项
   - 第二项
   - 第三项

   ## 代码

   ```rust
   fn main() {
       println!("hello");
   }
   ```
   ````

5. 点击发送

   响应是 PNG 图片二进制。Apifox 中可以切换到预览，或者下载响应保存为 `.png` 文件。

## curl 示例

使用项目中的 `test.md` 生成图片：

```powershell
curl.exe -X POST --data-binary "@test.md" http://localhost:3001/generate -o output.png
```

PowerShell 原生命令：

```powershell
Invoke-WebRequest `
  -Uri http://localhost:3001/generate `
  -Method POST `
  -InFile test.md `
  -OutFile output.png
```

生成结果：

```text
output.png
```

## Markdown 示例

````markdown
# 示例文档

这是一段中文 Markdown 内容。长中文段落会自动换行。

## 功能列表

- 支持标题
- 支持列表
- 支持代码块
- [x] 支持任务列表
- [ ] 待办事项

---

## 代码示例

```python
def greet(name: str) -> str:
    return f"Hello, {name}!"
```
````

## 样式说明

主题样式主要在：

```text
src/main.rs
```

相关位置：

- 布局尺寸：`IMAGE_WIDTH`、`PADDING`、`BODY_FONT_SIZE`、`LINE_HEIGHT`
- 主题颜色：`COLOR_SURFACE`、`COLOR_CARD`、`COLOR_TEXT`、`COLOR_SEED`
- 代码块样式：`add_code_block`
- 标题样式：`add_heading`
- 正文样式：`add_paragraph`
- 列表样式：`add_list_item`
- 字体栈：`build` 中的 SVG `<style>`

当前字体优先级：

```css
'0xProto Nerd Font Mono',
Microsoft YaHei,
SimHei,
Noto Sans CJK SC,
WenQuanYi Micro Hei,
PingFang SC,
Hiragino Sans GB,
monospace,
sans-serif
```

代码块字体优先级：

```css
'0xProto Nerd Font Mono',
Consolas,
Courier New,
monospace
```

## 注意事项

- `/generate` 接收的是 Markdown 原文，不是 JSON。
- 如果 Apifox 里看到乱码，通常是因为响应是 PNG 二进制，需要用图片预览或下载查看。
- 修改代码后，如果你运行的是 `target/release/markdown-preview-service.exe`，需要重新执行 `cargo build --release`。
- 当前渲染是手写 SVG 排版，不是浏览器排版引擎，因此复杂 Markdown/CSS 效果不会与浏览器完全一致。
