# 论文编号与文件夹命名

本文记录 PaperReader 导入论文时的编号规则，以及它和 Agentero Vault 文件夹名的关系。规则以已安装后端源码为准，不由 PDF 内容计算。

## 结论

论文编号由两部分组成：

```text
paper- + UUID v4 的前 10 位小写十六进制
```

例如 `paper-085e84f848`。后 10 位是随机唯一编号，不是 PDF、标题、路径、arXiv 编号或 DOI 的哈希。

## 代码位置

已安装后端：

`C:\Users\xuan\AppData\Local\Programs\PaperReader\resources\backend\reader_server.py`

两条导入路径使用同一规则：

| 函数 | 行 | 用途 |
| --- | --- | --- |
| `_import_pdf` | 1009 | 复制 PDF 后新建论文 |
| `_link_local_pdf` | 1058 | 为已有本地 PDF 建目录，不复制文件 |

生成语句：

```python
paper_id = f"paper-{uuid.uuid4().hex[:10]}"
```

`uuid.uuid4().hex` 是 32 位小写十六进制。这里只取前 10 位，因此格式固定为：

```text
paper-[0-9a-f]{10}
```

总长度是 16 个字符。

## 文件哈希的实际用途

导入时确实会计算整个 PDF 的 SHA-256，但它不参与命名。

`_pdf_digest` 在同文件第 889–891 行：

```python
def _pdf_digest(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()
```

这个摘要写入论文记录的 `sha256` 字段。`_import_pdf` 和 `_link_local_pdf` 用它与已有记录比较：摘要相同就视为重复论文，合并附加信息后返回，不再创建新编号。

因此：

| 数据 | 来源 | 用途 |
| --- | --- | --- |
| `paper-` 后 10 位 | `uuid.uuid4().hex[:10]` | 文件夹名和论文 ID |
| `sha256` | 整个 PDF 的 SHA-256 | 判断文件是否重复 |

PDF 的 SHA-256、SHA-1 或 MD5 前 10 位不应与文件夹后缀相同。

## Agentero 中的目录形式

Agentero 识别的论文文件夹沿用这个 ID：

```text
papers/<分类>/paper-<10 位十六进制>/
```

例如：

```text
papers/01-视觉地点识别/paper-085e84f848/
```

分类文件夹可以有任意层级。论文身份使用 Vault 相对路径，不只使用叶子文件夹名。

不符合 `paper-[0-9a-f]{10}` 的名字会被文件树当作普通目录。已经确认会展开成下一级文件夹的例子：

```text
papers/JEPA/paper-arxiv-2211-10831/
```

手工补录时应先生成不与现有 ID 冲突的 10 位编号，再同时写入：

- 文件夹名
- `.agentero/catalog.sqlite` 的 `papers.path` 与 `papers.id`
- `{paper}/.src/metadata.json` 的 `path` 与 `id`

不要用 arXiv 编号、标题或文件哈希拼文件夹名。
