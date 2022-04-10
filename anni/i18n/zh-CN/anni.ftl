## anni
anni-about = 为自建音乐站点构建的一整套工具
export-to = 导出内容存放的路径


## flac
flac = 提供 FLAC 处理相关的功能
flac-export = 导出内容
flac-export-type = 导出内容类型


## split
split = 提供音频分割相关的功能
split-format-input = 待切分音频的文件类型
split-format-output = 切分后输出音频的文件类型
split-no-apply-tags = 不将 CUE 中的元数据写入音频文件
split-no-import-cover = 不从切分目录寻找封面写入音频文件
split-output-file-exist = 输出路径下已存在文件 {$filename}，请删除文件后重试


## convention
convention = 提供定制化的音频检查约定检测
convention-check = 检查音频是否符合约定
convention-check-fix = 对不符合约定的音频文件进行修复


## repo
repo = 提供 Anni 元数据仓库的管理功能
repo-root = 需要管理的 Anni 元数据仓库根路径

repo-clone = 克隆元数据仓库
repo-clone-start = 准备克隆元数据仓库至{$path}...
repo-clone-done = 元数据仓库克隆完成

repo-add = 向元数据仓库中导入专辑
repo-add-edit = 在导入完成后打开文件编辑器
repo-invalid-album = 专辑目录格式错误：{$name}
repo-album-exists = 专辑 {$catalog} 已存在
repo-album-not-found = 不存在品番为 {$catalog} 的专辑
repo-album-info-mismatch = 专辑信息与专辑目录不一致

repo-validate-start = 仓库校验开始
repo-validate-end = 仓库校验结束
repo-validate-failed = 仓库校验失败
repo-catalog-filename-mismatch = 专辑 {$album_catalog} 的品番与文件名不一致
repo-invalid-artist = 艺术家名称不可用：{$artist}

repo-get = 从远程数据源获取专辑信息并导入
repo-get-print = 将获取的专辑信息输出到控制台而非导入

repo-edit = 当元数据仓库中存在该专辑时，打开仓库中对应的文件
repo-apply = 将元数据仓库中的数据应用到专辑
repo-validate = 检查仓库数据的合法性

repo-print = 根据品番输出元数据仓库中的数据
repo-print-type = 输出数据的类型
repo-print-clean = 省略 cue 输出中的 REM COMMENT "Generated by Anni"
repo-print-catalog = 输出信息的品番。通过后缀 '/{"{disc_id}"}' 指定需要输出信息的碟片编号，0 和 1 均代表第一张碟片

repo-db = 生成元数据仓库对应的数据库文件

repo-migrate = 迁移旧版本元数据仓库到新版本
repo-migrate-album-id = 为缺少 album_id 字段的专辑添加这一字段
