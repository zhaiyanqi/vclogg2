use gpui::{
    App, Background, Div, Hsla, Styled as _, div, hsla, linear_color_stop, linear_gradient, px, rgb,
};
use gpui_component::theme::{Theme, ThemeMode, ThemeTokens};

/// 界面表面使用叠在环境背景上的半透明材质。GPUI 没有元素级背景模糊，
/// 因此这里同时保存按
/// `c = fg * a + bg * (1 - a)` 逐层合成后的不透明值：ambient 背景 → 窗口材质 → 控件材质。
#[derive(Clone, Copy)]
pub(crate) struct ProductColors {
    pub(crate) background: Hsla,
    /// 标题栏、文件工具栏、标签栏与状态栏共用的窗口材质不透明回退色。
    /// 内容区不透明，所以只有这些材质需要不透明版本作为回退。
    pub(crate) header: Hsla,
    /// 环境背景底层 `linear-gradient(145deg, …)` 的两端。
    pub(crate) ambient_from: Hsla,
    pub(crate) ambient_to: Hsla,
    /// 压在底色上的三团光晕。GPUI 只有线性渐变，
    /// 这里改用从对应角落射出的斜向渐变近似，alpha 归零端与起点同色以免插值出灰边。
    /// 浅色主题没有第三团，`ambient_glow_c` 直接给全透明。
    pub(crate) ambient_glow_a: Hsla,
    pub(crate) ambient_glow_b: Hsla,
    pub(crate) ambient_glow_c: Hsla,
    /// 窗口与状态栏的半透明材质，靠 alpha 透出 ambient。
    pub(crate) material_heavy: Hsla,
    pub(crate) material_medium: Hsla,
    /// 各材质层上的斜向高光渐变。
    pub(crate) glass_sheen: Hsla,
    /// 材质顶棱的 1px 内高光，模拟受光的玻璃边缘。
    pub(crate) material_highlight: Hsla,
    pub(crate) surface: Hsla,
    /// 路径栏、file-meta、标签指示层与搜索控件共用的控件表面。
    pub(crate) control_surface: Hsla,
    pub(crate) foreground: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) muted_foreground: Hsla,
    pub(crate) border: Hsla,
    pub(crate) input: Hsla,
    /// 栏位之间的弱分隔线。
    pub(crate) divider: Hsla,
    pub(crate) primary: Hsla,
    pub(crate) primary_hover: Hsla,
    pub(crate) primary_active: Hsla,
    pub(crate) primary_foreground: Hsla,
    pub(crate) accent: Hsla,
    pub(crate) accent_foreground: Hsla,
    pub(crate) control: Hsla,
    pub(crate) control_hover: Hsla,
    pub(crate) control_active: Hsla,
    pub(crate) selection: Hsla,
    pub(crate) row_hover: Hsla,
    pub(crate) row_selected: Hsla,
    pub(crate) row_selected_border: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) danger_foreground: Hsla,
    pub(crate) warning: Hsla,
    pub(crate) warning_foreground: Hsla,
    pub(crate) success: Hsla,
    pub(crate) success_foreground: Hsla,
    pub(crate) info: Hsla,
    pub(crate) info_foreground: Hsla,
    pub(crate) scrollbar_thumb: Hsla,
    pub(crate) scrollbar_thumb_hover: Hsla,

    // 日志内容配色独立于 gpui-component 语义色，避免主题按钮或危险色的调整
    // 意外改变正文的命中与级别呈现。
    pub(crate) search_match: Hsla,
    pub(crate) search_match_foreground: Hsla,
    pub(crate) quick_find: Hsla,
    pub(crate) quick_find_foreground: Hsla,
    pub(crate) line_number: Hsla,
    pub(crate) line_number_background: Hsla,
    /// 未命中、未标记时的标记圆点描边。
    pub(crate) marker_border: Hsla,
    pub(crate) marker_matched: Hsla,
    pub(crate) marker_matched_border: Hsla,
    pub(crate) marker_marked: Hsla,
    pub(crate) marker_marked_border: Hsla,
    pub(crate) severity_error_background: Hsla,
    pub(crate) severity_error_accent: Hsla,
    pub(crate) severity_warning_background: Hsla,
    pub(crate) severity_warning_accent: Hsla,
    pub(crate) severity_info_background: Hsla,
    pub(crate) severity_info_accent: Hsla,
    pub(crate) severity_debug_background: Hsla,
    pub(crate) severity_debug_accent: Hsla,
}

/// Installs a complete product theme before any window renders. The persisted
/// preference may replace this mode later, but the first frame must never mix
/// gpui-component's stock palette with VCLogg2's product surfaces.
pub(crate) fn apply_product_theme(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    apply_product_colors(mode, cx);
}

fn color(value: u32) -> Hsla {
    rgb(value).into()
}

fn product_colors(mode: ThemeMode) -> ProductColors {
    if mode.is_dark() {
        ProductColors {
            // 合成底色取 ambient 深色渐变均值 #0f1425。
            background: color(0x0b1020),
            header: color(0x111827),
            ambient_from: color(0x090e1b),
            ambient_to: color(0x121427),
            ambient_glow_a: color(0x4a63dc).opacity(0.38),
            ambient_glow_b: color(0x1294ac).opacity(0.28),
            ambient_glow_c: color(0x7c3aed).opacity(0.22),
            material_heavy: color(0x121927).opacity(0.78),
            material_medium: color(0x141c2c).opacity(0.68),
            glass_sheen: hsla(0., 0., 1., 0.12),
            material_highlight: hsla(0., 0., 1., 0.07),
            surface: color(0x171d2a),
            control_surface: color(0x1f283a),
            foreground: color(0xf0f4fb),
            muted: color(0x1d2534),
            muted_foreground: color(0xbac4d5),
            border: color(0x283043),
            input: color(0x394156),
            divider: color(0x212739),
            primary: color(0x8b8cff),
            primary_hover: color(0x9d9eff),
            primary_active: color(0x7a7be8),
            primary_foreground: color(0x0b1020),
            accent: color(0x232748),
            accent_foreground: color(0xc9caff),
            control: color(0x1d2534),
            control_hover: color(0x273248),
            control_active: color(0x313d55),
            selection: color(0x284f7a),
            row_hover: color(0x212d41),
            row_selected: color(0x263f68),
            row_selected_border: color(0x4f87c7),
            danger: color(0xff8b9b),
            danger_foreground: color(0x2a0910),
            warning: color(0xe9a800),
            warning_foreground: color(0x1c1504),
            success: color(0x31b46d),
            success_foreground: color(0x06150f),
            info: color(0x4a9fe0),
            info_foreground: color(0x071522),
            scrollbar_thumb: color(0x526079),
            scrollbar_thumb_hover: color(0x71809b),

            search_match: color(0x3b6732),
            search_match_foreground: color(0xeff9e8),
            quick_find: color(0xf4c542),
            quick_find_foreground: color(0x201a06),
            line_number: color(0x8290a6),
            line_number_background: color(0x192231),
            marker_border: color(0x8794aa),
            marker_matched: color(0xe74856),
            marker_matched_border: color(0xd83b01),
            marker_marked: color(0xf5b942),
            marker_marked_border: color(0x8a5a00),
            severity_error_background: color(0x3b2025),
            severity_error_accent: color(0xd13438),
            severity_warning_background: color(0x3b321e),
            severity_warning_accent: color(0xe9a800),
            // info/debug 按 error/warning 的同一明度关系设置，避免在深色底上过亮。
            severity_info_background: color(0x1c2b3d),
            severity_info_accent: color(0x0f6cbd),
            severity_debug_background: color(0x242832),
            severity_debug_accent: color(0x7a828d),
        }
    } else {
        ProductColors {
            // 合成底色取 ambient 浅色渐变均值 #f0f1f2。
            background: color(0xeeeef0),
            header: color(0xf7f7f6),
            ambient_from: color(0xebeef3),
            ambient_to: color(0xe9eef5),
            ambient_glow_a: color(0xbfdbfe).opacity(0.56),
            ambient_glow_b: color(0xfed7aa).opacity(0.42),
            ambient_glow_c: color(0xfed7aa).opacity(0.),
            material_heavy: color(0xfcfbf8).opacity(0.62),
            material_medium: color(0xfaf9f6).opacity(0.52),
            glass_sheen: hsla(0., 0., 1., 0.12),
            material_highlight: hsla(0., 0., 1., 0.8),
            surface: color(0xfbfaf8),
            control_surface: color(0xfcfcfb),
            foreground: color(0x20242d),
            muted: color(0xf2f0ec),
            muted_foreground: color(0x606876),
            border: color(0xd0d3d7),
            input: color(0xbdc0c6),
            divider: color(0xdbdcdf),
            primary: color(0x2563eb),
            primary_hover: color(0x2159d4),
            primary_active: color(0x1e4fbc),
            primary_foreground: color(0xffffff),
            accent: color(0xdae1f1),
            accent_foreground: color(0x1d4ed8),
            control: color(0xfbfaf8),
            control_hover: color(0xe9e7e2),
            control_active: color(0xdfddd7),
            selection: color(0xcfe0ff),
            row_hover: color(0xf0f3f8),
            row_selected: color(0xdce9ff),
            row_selected_border: color(0x7db7e8),
            danger: color(0xbd2638),
            danger_foreground: color(0xffffff),
            warning: color(0xe9a800),
            warning_foreground: color(0xffffff),
            success: color(0x31b46d),
            success_foreground: color(0xffffff),
            info: color(0x0f6cbd),
            info_foreground: color(0xffffff),
            scrollbar_thumb: color(0xa1a7b3),
            scrollbar_thumb_hover: color(0x717b8e),

            search_match: color(0xc8efa8),
            search_match_foreground: color(0x10200d),
            quick_find: color(0xf4c542),
            quick_find_foreground: color(0x201a06),
            line_number: color(0x747c88),
            line_number_background: color(0xf1f0ed),
            marker_border: color(0x858c98),
            marker_matched: color(0xe74856),
            marker_matched_border: color(0xd83b01),
            marker_marked: color(0xf5b942),
            marker_marked_border: color(0x8a5a00),
            severity_error_background: color(0xfff0f1),
            severity_error_accent: color(0xd13438),
            severity_warning_background: color(0xfff8e8),
            severity_warning_accent: color(0xe9a800),
            severity_info_background: color(0xeff7ff),
            severity_info_accent: color(0x0f6cbd),
            severity_debug_background: color(0xf5f6f8),
            severity_debug_accent: color(0x7a828d),
        }
    }
}

/// 当前生效主题的完整产品配色。呈现代码读这里而不是自己拼语义色，
/// 保证正文、当前结果与全局结果三处对同一语义使用同一个值。
pub(crate) fn palette(cx: &App) -> ProductColors {
    product_colors(if Theme::global(cx).is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    })
}

fn apply_product_colors(mode: ThemeMode, cx: &mut App) {
    let colors = product_colors(mode);
    let theme = Theme::global_mut(cx);

    // 15px keeps the desktop shell compact while all rem-based controls,
    // typography and spacing continue to zoom as one coherent scale.
    theme.font_size = px(15.);
    theme.mono_font_size = px(13.);

    theme.background = colors.background;
    theme.foreground = colors.foreground;
    theme.popover = colors.surface;
    theme.popover_foreground = colors.foreground;
    theme.group_box = colors.surface;
    theme.group_box_foreground = colors.foreground;
    theme.muted = colors.muted;
    theme.muted_foreground = colors.muted_foreground;
    theme.border = colors.border;
    theme.input = colors.input;

    theme.primary = colors.primary;
    theme.primary_hover = colors.primary_hover;
    theme.primary_active = colors.primary_active;
    theme.primary_foreground = colors.primary_foreground;
    theme.accent = colors.accent;
    theme.accent_foreground = colors.accent_foreground;
    theme.ring = colors.primary;
    theme.caret = colors.primary;
    theme.selection = colors.selection;
    theme.drag_border = colors.primary;
    theme.drop_target = colors.accent;

    theme.button = colors.control;
    theme.button_hover = colors.control_hover;
    theme.button_active = colors.control_active;
    theme.button_foreground = colors.foreground;
    theme.button_primary = colors.primary;
    theme.button_primary_hover = colors.primary_hover;
    theme.button_primary_active = colors.primary_active;
    theme.button_primary_foreground = colors.primary_foreground;
    theme.secondary = colors.control;
    theme.secondary_hover = colors.control_hover;
    theme.secondary_active = colors.control_active;
    theme.secondary_foreground = colors.foreground;
    theme.button_secondary = colors.control;
    theme.button_secondary_hover = colors.control_hover;
    theme.button_secondary_active = colors.control_active;
    theme.button_secondary_foreground = colors.foreground;

    theme.colors.list = colors.surface;
    theme.list_even = colors.background;
    theme.list_head = colors.muted;
    theme.list_hover = colors.row_hover;
    theme.list_active = colors.row_selected;
    theme.list_active_border = colors.row_selected_border;
    theme.table = colors.surface;
    theme.table_even = colors.background;
    theme.table_head = colors.muted;
    theme.table_head_foreground = colors.muted_foreground;
    theme.table_foot = colors.muted;
    theme.table_foot_foreground = colors.muted_foreground;
    theme.table_hover = colors.row_hover;
    theme.table_active = colors.row_selected;
    theme.table_active_border = colors.row_selected_border;
    theme.table_row_border = colors.border;

    theme.tab_bar = colors.header;
    theme.tab_bar_segmented = colors.header;
    theme.tab = colors.header;
    theme.tab_foreground = colors.muted_foreground;
    theme.tab_active = colors.control_surface;
    theme.tab_active_foreground = colors.primary;
    theme.title_bar = colors.header;
    theme.title_bar_border = colors.border;
    theme.status_bar = colors.header;
    theme.status_bar_border = colors.border;
    theme.sidebar = colors.muted;
    theme.sidebar_foreground = colors.foreground;
    theme.sidebar_border = colors.border;
    theme.sidebar_accent = colors.accent;
    theme.sidebar_accent_foreground = colors.accent_foreground;
    theme.sidebar_primary = colors.primary;
    theme.sidebar_primary_foreground = colors.primary_foreground;

    theme.danger = colors.danger;
    theme.danger_hover = colors.danger.opacity(0.88);
    theme.danger_active = colors.danger.opacity(0.76);
    theme.danger_foreground = colors.danger_foreground;
    theme.warning = colors.warning;
    theme.warning_hover = colors.warning.opacity(0.88);
    theme.warning_active = colors.warning.opacity(0.76);
    theme.warning_foreground = colors.warning_foreground;
    theme.success = colors.success;
    theme.success_hover = colors.success.opacity(0.88);
    theme.success_active = colors.success.opacity(0.76);
    theme.success_foreground = colors.success_foreground;
    theme.info = colors.info;
    theme.info_hover = colors.info.opacity(0.88);
    theme.info_active = colors.info.opacity(0.76);
    theme.info_foreground = colors.info_foreground;

    theme.scrollbar = colors.muted;
    theme.scrollbar_thumb = colors.scrollbar_thumb;
    theme.scrollbar_thumb_hover = colors.scrollbar_thumb_hover;

    theme.tokens = ThemeTokens::from(&theme.colors);
    // `tokens.background` 是 Segmented 标签指示层、Kbd 与若干浮层的填充色。工作区根节点
    // 自己用 `theme.background` 铺满窗口，所以这里把 token 指向控件材质，让活动标签
    // 比标签栏更亮一档，而不是与窗口底色相同。
    theme.tokens.background = colors.control_surface.into();
    Theme::sync_base(cx);
}

/// 窗口最底层的环境色，使用 `linear_gradient(145deg, …)`。
/// 光晕由 [`ambient_glow_layers`] 另外压在上面。
pub(crate) fn ambient_base(colors: &ProductColors) -> Background {
    linear_gradient(
        145.,
        linear_color_stop(colors.ambient_from, 0.),
        linear_color_stop(colors.ambient_to, 1.),
    )
}

/// 压在 [`ambient_base`] 上的三团角落光晕，必须作为窗口根节点的头几个子元素铺开，
/// 才能落在所有界面内容之下。根节点需要是 `relative`；这些层绝对定位、不占布局。
///
/// 每层都从一个角落射向对角，终点是同色的全透明——GPUI 在 sRGB 空间插值，
/// 用通用 transparent 会让渐变中段掉向灰色。
pub(crate) fn ambient_glow_layers(colors: &ProductColors) -> [Div; 3] {
    let glow = |color: Hsla, angle: f32, reach: f32| {
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .bg(linear_gradient(
                angle,
                linear_color_stop(color, 0.),
                linear_color_stop(color.opacity(0.), reach),
            ))
    };
    [
        glow(colors.ambient_glow_a, 135., 0.52),
        glow(colors.ambient_glow_b, 225., 0.48),
        glow(colors.ambient_glow_c, 350., 0.58),
    ]
}

/// 标题栏、文件工具栏、标签栏与搜索栏共用的窗口材质。
///
/// GPUI 没有元素级背景模糊，取样不到材质下方的画面。这里直接把半透明色叠在
/// ambient 平滑渐变上；真正需要模糊的只有压着日志正文的浮层。
pub(crate) fn header_material(colors: &ProductColors) -> Hsla {
    colors.material_heavy
}

/// 状态栏使用的较薄材质。
pub(crate) fn footer_material(colors: &ProductColors) -> Hsla {
    colors.material_medium
}

/// 材质表面那道斜向反光。调用方的容器必须是 `relative`，本元素绝对定位、不占布局。
pub(crate) fn glass_sheen_layer(colors: &ProductColors) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(linear_gradient(
            115.,
            linear_color_stop(colors.glass_sheen, 0.),
            linear_color_stop(colors.glass_sheen.opacity(0.), 0.42),
        ))
}

/// 覆在材质顶棱的 1px 内高光。调用方的容器必须是 `relative`，本元素绝对定位、不占布局。
pub(crate) fn material_highlight_line(colors: &ProductColors) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(1.))
        .bg(colors.material_highlight)
}

pub(crate) fn text_selection_highlight(cx: &App) -> Hsla {
    let colors = palette(cx);
    colors.primary.opacity(if Theme::global(cx).is_dark() {
        0.34
    } else {
        0.26
    })
}

pub(crate) fn suggestion_match_highlight(cx: &App) -> Hsla {
    palette(cx)
        .quick_find
        .opacity(if Theme::global(cx).is_dark() {
            0.38
        } else {
            0.28
        })
}
