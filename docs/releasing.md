# 发布 strayd

正式 npm 发布由 [build-npm.yml](../.github/workflows/build-npm.yml) 完成：版本校验 → 六平台原生构建与测试 → 统一打包 → 六平台安装及终端验收 → npm OIDC 发布。发布的是各平台共同验收过的同一个 `.tgz`，不会在发布步骤重新打包。

## 发布一个版本

1. 更新根 `package.json`、`npm/strayd/package.json`、`crates/port-deck-cli/Cargo.toml` 中的版本，用 `cargo check --workspace` 同步 `Cargo.lock`，并更新 npm README 中固定版本的资源链接。
2. 运行 `bun run check`、`bun run test` 和 `bun run test:release`，提交并推送到 `main`。
3. 为该提交创建与包版本完全一致的正式标签，例如：

   ```bash
   git tag -a v0.2.4 -m 'Release v0.2.4'
   git push origin refs/tags/v0.2.4
   ```

工作流要求标签提交位于 `main` 历史中，只接受正式 `X.Y.Z` 版本。上游仓库的自动发布仅允许所有者 `MarioJames` 触发或重新运行。任一平台失败都会阻断发布。

重复运行时，只有 npm 上已有版本的 SHA-512 与当前安装包完全一致才会跳过发布；同版本不同内容会报错，必须升版。已经推送的标签不应移动。

普通手动构建可运行 `gh workflow run build-npm.yml --repo MarioJames/strayd --ref main`。这只构建和验收，不发布 `main` 分支中的内容。

## 一次性配置

GitHub 仓库的 `npm` Environment 只允许 `main` 分支和 `v*` 标签。`main` 与 `v*` 的服务端 ruleset 限制更新者/创建者为所有者；这些设置不保存在 YAML 中，fork 不会继承。其他人可以提交 PR，但不能直接改主分支或创建发布标签。不要把仓库管理权限授予不可信账号：管理员可以修改仓库的保护规则。

在 npm 的 `strayd` 包 Settings → Trusted publishing 中绑定：

| 字段 | 值 |
| --- | --- |
| Provider | GitHub Actions |
| Organization or user | `MarioJames` |
| Repository | `strayd` |
| Workflow filename | `build-npm.yml` |
| Environment | `npm` |
| Allowed action | `npm publish` |

也可以用 npm 11.15+ 的 CLI 配置。以下命令需要一次账号双重验证；凭据由 npm 管理，不写入仓库：

```bash
npm trust github strayd --repo MarioJames/strayd --file build-npm.yml --env npm --allow-publish --yes
```

只有发布与授权检查 job 拥有 `id-token: write` 权限；构建和测试 job 不持有发布身份。npm 自动交换 GitHub 的短期 OIDC 身份并生成 provenance，不需要设置 `NPM_TOKEN` 或 `NODE_AUTH_TOKEN`。

配置后可以单独验证信任关系，不构建或发布任何版本：

```bash
gh workflow run build-npm.yml --repo MarioJames/strayd --ref main -f check_oidc=true
```

这个检查实际向 npm 交换 `strayd` 的短期发布凭据，拒绝访问或缺少凭据都算失败；不会把令牌写入日志或文件。`npm publish --dry-run` 在没有有效凭据时也可能成功，不能替代此验证。

## 本机检查与手动备用方式

```bash
bun scripts/release-npm.ts --validate
bun scripts/release-npm.ts --check /path/to/artifacts
bun scripts/release-npm.ts --verify /path/to/artifacts
```

三个命令分别检查版本一致性、完整六平台安装包、npm 上已发布内容的 SHA-512。目录中必须只有一个 `.tgz`。`bun run pack:cli` 只包含当前宿主架构，不是正式发布包。

确需本机手动发布时，先下载工作流验收成功的 `.tgz`，完成以上检查，再运行 npm 的交互式发布：

```bash
npm login --auth-type=legacy
npm publish /path/to/strayd-X.Y.Z.tgz --access=public --tag=latest --ignore-scripts --auth-type=legacy
```

凭据保存在用户级 `~/.npmrc`，双重验证码按 npm 的提示输入；不把 token、密码、验证码或 TOTP 密钥写进脚本、命令参数或仓库。`npm login` 会话有有效期，脚本不能延长它；日常发布使用 OIDC。

参考：[npm Trusted Publishing](https://docs.npmjs.com/trusted-publishers/)、[npm trust](https://docs.npmjs.com/cli/v11/commands/npm-trust/)、[GitHub rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets)。
