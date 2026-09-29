# 配置 OpenAI 兼容翻译

Agentero 的「OpenAI 兼容」翻译用于接入支持 **Chat Completions** 格式的模型服务。它适合 OpenAI、DeepSeek、Kimi、Qwen 兼容模式、SiliconFlow、智谱等把聊天接口做成 OpenAI 兼容形态的服务；不适合只提供厂商自定义文本接口的服务。

## 填写位置

打开 **Settings → 翻译 → API 服务 → OpenAI 兼容**：

| 字段 | 填写方式 |
|---|---|
| API Key | 服务商控制台生成的密钥。只填在设置页，不要写进 Vault、文档、终端历史或 Git。 |
| 端点 | 填 **Base URL**，不是完整请求 URL。例如 `https://api.openai.com/v1`。 |
| 模型 | 服务商实际支持的模型 ID，例如 `gpt-4.1-mini`、`deepseek-chat`。 |

填完点「确定」。设置页会做一次短 probe；probe 通过后，把「默认服务」切到「OpenAI 兼容」即可用于 PDF 划词翻译和全文翻译。

## 接口要求

Agentero 会把端点拼成：

```text
POST {baseUrl}/chat/completions
```

请求体是 OpenAI Chat Completions 风格：

```json
{
  "model": "<model>",
  "messages": [
    { "role": "system", "content": "..." },
    { "role": "user", "content": "..." }
  ],
  "temperature": 0.2
}
```

返回值必须能从这里取到译文：

```text
choices[0].message.content
```

因此设置里的端点应是根地址：

| 服务 | Base URL 示例 |
|---|---|
| OpenAI | `https://api.openai.com/v1` |
| DeepSeek | `https://api.deepseek.com` |
| Kimi / Moonshot | `https://api.moonshot.ai/v1` |
| Qwen 兼容模式 | `https://maas.qwencloudapi.com/compatible-mode/v1` |
| SiliconFlow | `https://api.siliconflow.cn/v1` |
| 智谱 BigModel | `https://open.bigmodel.cn/api/paas/v4` |

不要填写这些值：

| 错误写法 | 原因 |
|---|---|
| `https://api.openai.com/v1/chat/completions` | Agentero 已经会追加 `/chat/completions`。 |
| `https://api.example.com/v1/responses` | Responses API 不是 Chat Completions。 |
| `https://api.example.com/v1/completions` | 旧 Completions API 没有 `messages` / `choices[0].message.content`。 |
| `https://api.minimax.cn/v1/text/chatcompletion_v2` | 这是 MiniMax 自定义文本接口，不是 OpenAI Chat Completions 兼容 Base URL。 |

## MiniMax M3 注意事项

MiniMax 中国区 M3 官方示例当前使用：

```text
POST https://api.minimax.cn/v1/text/chatcompletion_v2
model: MiniMax-M3
```

这个路径不是 OpenAI Chat Completions 的 `/chat/completions` 形态。把 `https://api.minimax.cn/v1` 或 `https://api.minimaxi.com/v1` 填到 Agentero 后，应用会请求 `/chat/completions`，因此 probe 可能一直失败。

如果服务商提供了单独的 OpenAI 兼容入口，可以使用那个入口的 Base URL；如果只提供 `text/chatcompletion_v2`，当前版本不能通过「OpenAI 兼容」翻译接入，需要后续增加专门的 MiniMax provider 或支持完整 endpoint 模式。

## Probe 失败排查

设置页 probe 是 5 秒短请求，只用于快速判断配置是否大体可用。失败时按下面顺序检查：

1. **区域是否匹配**：中国区和国际区账号、Key、域名可能不互通。中国区通常用 `.cn` / `.com` 域名，国际区通常用 `.io` 域名，以服务商控制台为准。
2. **端点是否是 Base URL**：不要带 `/chat/completions`、`/responses`、`/text/chatcompletion_v2`。
3. **模型名是否精确**：模型 ID 区分服务商和区域，照控制台或模型文档填写。
4. **Key 是否有效**：检查余额、套餐、权限、RPM / TPM 限制；`401` / `403` 通常是 Key、区域或权限问题。
5. **网络与代理**：Settings → General → Network proxy 会影响 Host 发出的翻译请求。
6. **超时或限流**：probe 只有 5 秒，真实翻译默认 30 秒；高峰期可能 probe 失败但正式请求偶尔成功。

如果你把 API Key 发到了聊天、Issue、日志或截图里，立即去服务商控制台撤销并重建。Agentero 设置页会把已保存的 Key 掩码显示，但外部泄露的 Key 无法由应用补救。

