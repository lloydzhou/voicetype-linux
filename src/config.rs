//! Configuration: ~/.config/voicetype/config.toml

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Supported UI/ASR languages: (config key, display name, whisper code)
pub const LANGUAGES: [(&str, &str, &str); 5] = [
    ("en", "English", "en"),
    ("zh-Hans", "简体中文", "zh"),
    ("zh-Hant", "繁體中文", "zh"),
    ("ja", "日本語", "ja"),
    ("ko", "한국어", "ko"),
];

pub fn language_index(key: &str) -> usize {
    LANGUAGES
        .iter()
        .position(|(k, _, _)| *k == key)
        .unwrap_or(1) // default zh-Hans
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub language: String,
    pub llm: LlmConfig,
    pub model_path: String,
    /// ASR engine: "sensevoice" (SenseVoiceSmall INT8 via sherpa-onnx, live
    /// path) or "whisper" (legacy chunked whisper.cpp path).
    #[serde(default = "default_engine")]
    pub engine: String,
    /// Directory holding the SenseVoice ONNX model (model.int8.onnx +
    /// tokens.txt), as downloaded by `make model-sensevoice`.
    #[serde(default = "default_sensevoice_dir")]
    pub sensevoice_dir: String,
    /// Display-only description of the (hard-coded) hotkey.
    #[serde(default = "default_hotkey_display")]
    pub hotkey: String,
    /// Active hold-to-talk chords, e.g. ["RightCtrl+RightShift"]. Empty or
    /// missing selects the default. Parsed by hotkey::parse_combos, which
    /// fails startup on unknown key names.
    #[serde(default)]
    pub hotkeys: Vec<String>,
    /// Switch fcitx/ibus to English before paste and restore after.
    /// Default false: never touch the IME.
    #[serde(default)]
    pub ime_switch: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "zh-Hans".to_string(),
            llm: LlmConfig::default(),
            model_path: default_model_path().to_string_lossy().into_owned(),
            engine: default_engine(),
            sensevoice_dir: default_sensevoice_dir(),
            hotkey: default_hotkey_display(),
            hotkeys: Vec::new(),
            ime_switch: false,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("voicetype")
        .join("config.toml")
}

pub fn default_model_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("voicetype")
        .join("models")
        .join("ggml-base.bin")
}

pub fn default_engine() -> String {
    "sensevoice".to_string()
}

pub fn default_hotkey_display() -> String {
    "RightCtrl+RightShift (hold to talk)".to_string()
}

pub fn default_sensevoice_dir() -> String {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("voicetype")
        .join("models")
        .join("sensevoice")
        .to_string_lossy()
        .into_owned()
}

impl Config {
    pub fn load_or_create() -> Config {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("config parse failed ({e}); using defaults");
                    Config::default()
                }
            },
            Err(_) => {
                let c = Config::default();
                if let Err(e) = c.save() {
                    log::warn!("cannot write default config {}: {e}", path.display());
                } else {
                    log::info!("created default config at {}", path.display());
                }
                c
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}

/// Shared handle used across UI / tray / worker threads.
pub type SharedConfig = Arc<Mutex<Config>>;

pub fn shared(config: Config) -> SharedConfig {
    Arc::new(Mutex::new(config))
}

static S2T: OnceLock<std::collections::HashMap<char, char>> = OnceLock::new();
static T2S: OnceLock<std::collections::HashMap<char, char>> = OnceLock::new();

/// The Simplified->Traditional pair table as (simplified, traditional) pairs.
fn s2t_pairs() -> Vec<(char, char)> {
    const PAIRS: &str = concat!(
            "爱愛碍礙袄襖罢罷摆擺败敗办辦帮幫宝寶报報币幣边邊变變标標表錶别別宾賓补補",
            "参參惨慘灿燦苍蒼层層尝嘗长長偿償厂廠车車彻徹尘塵陈陣称稱诚誠迟遲冲衝",
            "丑醜出齣础礎处處触觸传傳疮瘡闯闖创創纯純词詞辞辭聪聰从從丛叢凑湊窜竄",
            "错錯达達带帶贷貸单單担擔胆膽弹彈当當党黨岛島祷禱灯燈邓鄧敌敵涤滌递遞",
            "点點电電垫墊淀澱调調雕鵰钓釣东動动動冻凍栋棟斗鬥独獨读讀断斷队隊对對",
            "吨噸夺奪堕墮鹅鵝额額恶惡儿兒尔爾饵餌发發罚罰阀閥范範饭飯访訪纺紡飞飛",
            "废廢费費纷紛奋奮粪糞丰豐风風疯瘋冯馳凤鳳佛彿妇婦复復复複负負讣訃",
            "该該盖蓋干幹赶趕冈岡纲綱钢鋼岗崗刚剛糕餻阁閣个個给給巩鞏共供求宮恭",
            "沟溝构構购購够夠蛊蠱顾顧关關观觀馆館惯慣贯貫广廣归歸规規龟龜国國过過",
            "骇駭汉漢号號合閡鹤鶴贺賀横橋轰轟红紅后後厚垕壶壺护護沪滬华華划劃画畫",
            "话話怀懷坏壞欢歡环環还還换換唤喚黄黃回迴汇彙会會绘繪浑渾伙夥获獲",
            "击擊机機积積饥饑鸡雞极極级級几幾计計记記际際剂劑济濟继繼家傢价價驾駕",
            "歼殲监監坚堅艰艱检檢简簡见見荐薦舰艦鉴鑑践踐键鍵将將浆漿讲講奖獎",
            "胶膠阶階节節洁潔结結诫誡届屆紧緊仅僅进進劲勁惊驚经經竞競净淨静靜",
            "旧舊剧劇惧懼据據剧劇卷捲决決绝絕军軍均勻开開凯凱慨嘅壳殼课課垦墾",
            "恳懇夸誇块塊亏虧困睏扩擴阔闊蜡蠟来來赖賴蓝藍栏欄拦攔篮籃烂爛劳勞",
            "涝澇乐樂垒壘类類泪淚篱籬里裡礼禮丽麗厉厲励勵历歷历曆隶隸联聯怜憐",
            "莲蓮练練炼煉粮糧两兩辆輛辽遼疗療猎獵临臨邻鄰鳞鱗灵靈岭嶺领領刘劉",
            "龙龍楼樓娄婁乱亂论論罗羅络絡骆駱妈媽马馬码碼吗嗎买買卖賣迈邁麦麥",
            "脉脈满滿猫貓么麼门門闷悶们們萌甿梦夢弥彌谜謎米冪绵綿缅緬面麵庙廟",
            "灭滅悯憫鸣鳴谬謬谋謀亩畝纳納难難恼惱脑腦闹鬧内內拟擬腻膩酿釀鸟鳥",
            "聂聶宁寧柠檸农農浓濃纽紐疟瘧盘盤庞龐赔賠佩珮喷噴鹏鵬骗騙飘飄频頻",
            "贫貧苹蘋评評凭憑泼潑铺鋪仆僕朴樸谱譜齐齊骑騎岂豈启啟弃棄气氣讫訖",
            "迁遷佥僉牵牽铅鉛浅淺谴譴欠欿枪槍强強抢搶桥橋乔喬窃竊钦欽亲親侵寢",
            "轻輕倾傾请請庆慶琼瓊穷窮秋鞦区區驱驅曲麯趋趨龋齲权權劝勸却郤确確",
            "让讓饶饒扰擾热熱认認荣榮绒絨软軟锐銳润潤洒灑萨薩赛賽伞傘丧喪涩澀",
            "杀殺沙紗筛篩晒曬删刪闪閃陕陝伤傷赏賞烧燒绍紹设設绅紳审審肾腎声聲",
            "胜勝绳繩圣聖师師湿濕诗詩时時识識实實势勢视視试試适適饰飾释釋寿壽",
            "兽獸书書属屬术術树樹竖豎数數帅帥双雙谁誰顺順说說硕碩丝絲松鬆苏蘇",
            "诉訴虽雖随隨岁歲孙孫损損缩縮锁鎖台臺态態摊攤谈談坛壇叹嘆汤湯烫燙",
            "涛濤涛濤腾騰誊謄体體条條铁鐵厅廳听聽统統头頭图圖涂塗团團推頹颓頹",
            "退脫脱脫驮馱洼窪袜襪弯彎万萬网網卫衛为為围圍维維伟偉纬緯卫衛温溫",
            "闻聞稳穩问問涡渦无無务務雾霧误誤牺犧习習戏戲系係细細虾蝦吓嚇峡峽",
            "侠俠狭狹鲜鮮纤纖贤賢闲閒显顯险險现現献獻县縣线線宪憲陷馅餡乡鄉",
            "响響项項象像萧蕭销銷晓曉协協胁脅挟挾谐諧写寫泻瀉泄洩兴興须須许許",
            "续續选選悬懸学學勋勳询詢寻尋训訓讯訊逊遜压壓鸦鴉鸭鴨哑啞亚亞烟煙",
            "盐鹽严嚴颜顏阎閻艳豔厌厭验驗谚諺阳陽杨楊养養样樣痒癢谣謠药藥爷爺",
            "页頁业業叶葉医醫仪儀义義议議亿億忆憶异異译譯阴陰银銀隐隱应應婴嬰",
            "樱櫻鹰鷹迎迊赢贏影穎忧憂优優犹猶邮郵娱娛渔漁屿嶼与與语語誉譽",
            "预預驭馭鸳鴛渊淵园園员員圆圓缘緣源遠远遠愿願约約跃躍钥鑰云雲",
            "运運晕暈韵韻杂雜灾災载載赞讚脏臟凿鑿枣棗责責择擇则則泽澤贼賊赠贈",
            "扎劄札劄闸閘诈詐斋齋债債战戰张張涨漲帐帳账賬障幛折摺这這针針侦偵",
            "珍珎镇鎮阵陣争爭征徵挣掙症癥证證织織职職执執纸紙挚摯掷擲质質",
            "钟鐘终終种種众眾皱皺昼晝猪豬铸鑄筑築专專转轉赚賺桩樁装裝壮壯",
            "状狀锥錐谆諄浊濁资資渍漬踪蹤综綜总總纵縱邹鄒诅詛组組钻鑽醉酔",
            "尊遵遵昨拝",
        );
        let chars: Vec<char> = PAIRS.chars().collect();
        let mut out = Vec::with_capacity(chars.len() / 2);
        let mut i = 0;
        while i + 1 < chars.len() {
            out.push((chars[i], chars[i + 1]));
            i += 2;
        }
        out
}

/// Simplified -> Traditional conversion table (common characters, approximation).
pub fn s2t_char_map() -> &'static std::collections::HashMap<char, char> {
    S2T.get_or_init(|| s2t_pairs().into_iter().collect())
}

/// Extra Traditional->Simplified pairs, format: traditional, simplified, ...
/// Covers very common traditional chars missing from the legacy S2T table
/// (like 動/東) and overrides the few bogus pairs in it.
const T2S_EXTRA: &str = concat!(
    "動动東东門门關关開开時时來来說说學学國国会會後后長长車车馬马鳥鸟",
    "語语書书寫写讀读這这對对們们間间覺觉腳脚陳陈陣阵馳驰穎颖頹颓紗纱",
    "餡馅遠远勻匀鏡镜徑径盡尽儘尽像像昨昨宮宫供供冪幂鬚须鹹咸嚮向隻只",
    "週周註注準准製制閡阂爾尔爲为髮发藝艺營营趙赵錢钱詞词題题錄录魚鱼",
    "離离覽览貴贵贊赞裏里並并嘗尝錶表嚇吓戲戏壓压補补廢废費费顯显竊窃",
    "遲迟幹干歷历鐘钟點点聽听頭头務务識识雙双難难雲云電电號号邊边還还",
    "過过麼么裡里個个兒儿無无買买賣卖葉叶龍龙鳳凤雖虽當当歸归錯错為为",
    "萬万與与劃划計计應应觀观歡欢樂乐氣气數数處处據据樣样機机經经發发",
    "體体條条運运區区",
);

/// Traditional -> Simplified conversion table: inverse of the S2T table
/// plus common extra pairs. Whisper tends to emit Traditional for zh, so
/// zh-Hans output needs this direction.
pub fn t2s_char_map() -> &'static std::collections::HashMap<char, char> {
    T2S.get_or_init(|| {
        let mut m: std::collections::HashMap<char, char> = s2t_pairs()
            .into_iter()
            .map(|(s, t)| (t, s))
            .collect();
        let chars: Vec<char> = T2S_EXTRA.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            // (traditional, simplified) -> traditional maps to simplified
            m.insert(chars[i], chars[i + 1]);
            i += 2;
        }
        m
    })
}
