#include "editor_theme.h"
#include "editor_style.h"

#include <QApplication>
#include <QEvent>
#include <QGuiApplication>
#include <QHash>
#include <QJsonArray>
#include <QPointer>
#include <QStyleHints>
#include <QWidget>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// The light or dark value of a token.
QColor tone(bool dark, const char *light, const char *darkValue) { return cssColor(dark ? darkValue : light); }
QColor hex(const char *css) { return cssColor(css); }

// The same in 1% steps, for a colour that should move no further than it must.
QColor justReadableOn(const QColor &color, const QColor &background, qreal ratio) {
    return readableOn(color, background, ratio, 100);
}

ClipStyle clipStyle(const char *fill, const char *header, const char *border, const char *label, const char *wave) {
    return {QColor(fill), QColor(header), QColor(border), QColor(label), QColor(wave)};
}

struct AppearanceState {
    ColorScheme scheme = environmentColorScheme();
    EditorTheme theme;
    bool computed = false;
    bool applied = false;
    bool applying = false;
};

AppearanceState &state() {
    static AppearanceState s;
    return s;
}

// Re-applies on palette changes, which is how an appearance change arrives
// where Qt has no colour scheme to report.
class PaletteWatcher final : public QObject {
public:
    using QObject::QObject;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched == qApp && event->type() == QEvent::ApplicationPaletteChange) applyEditorAppearance();
        return QObject::eventFilter(watched, event);
    }
};

} // namespace

bool systemIsDark() {
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    const Qt::ColorScheme scheme = QGuiApplication::styleHints()->colorScheme();
    if (scheme != Qt::ColorScheme::Unknown) return scheme == Qt::ColorScheme::Dark;
#endif
    return QGuiApplication::palette().color(QPalette::Window).lightness() < 128;
}

ColorScheme environmentColorScheme() {
    const QByteArray forced = qgetenv("KERF_COLOR_SCHEME").trimmed().toLower();
    if (forced == "light") return ColorScheme::Light;
    if (forced == "dark") return ColorScheme::Dark;
    return ColorScheme::System;
}

bool schemeIsDark(ColorScheme scheme) {
    return scheme == ColorScheme::Dark || (scheme == ColorScheme::System && systemIsDark());
}

QColor readableOn(const QColor &color, const QColor &background, qreal ratio, int steps) {
    const QColor target = background.lightness() < 128 ? QColor(Qt::white) : QColor(Qt::black);
    QColor result = color;
    for (int step = 1; step <= steps && contrastRatio(result, background) < ratio; ++step)
        result = mixColor(color, target, qreal(step) / steps);
    return result;
}

QColor cssColor(const char *css) {
    const QString name = QString::fromLatin1(css);
    if (name.size() != 9) return QColor(name);
    QColor color(name.left(7));
    color.setAlpha(name.mid(7, 2).toInt(nullptr, 16));
    return color;
}

QColor mixColor(const QColor &under, const QColor &over, qreal amount) {
    return QColor::fromRgbF(under.redF() + (over.redF() - under.redF()) * amount,
                            under.greenF() + (over.greenF() - under.greenF()) * amount,
                            under.blueF() + (over.blueF() - under.blueF()) * amount,
                            under.alphaF() + (over.alphaF() - under.alphaF()) * amount);
}

qreal contrastRatio(const QColor &a, const QColor &b) {
    const auto luminance = [](const QColor &color) {
        const auto channel = [](qreal v) { return v <= 0.03928 ? v / 12.92 : std::pow((v + 0.055) / 1.055, 2.4); };
        return 0.2126 * channel(color.redF()) + 0.7152 * channel(color.greenF()) + 0.0722 * channel(color.blueF());
    };
    const qreal la = luminance(a);
    const qreal lb = luminance(b);
    return (std::max(la, lb) + 0.05) / (std::min(la, lb) + 0.05);
}

QColor readableAlphaOn(const QColor &color, const QColor &background, qreal ratio) {
    QColor result = color;
    const auto laid = [&background](QColor top) {
        const qreal alpha = top.alphaF();
        top.setAlpha(255);
        return mixColor(background, top, alpha);
    };
    while (result.alpha() < 255 && contrastRatio(laid(result), background) < ratio) result.setAlpha(result.alpha() + 1);
    return result;
}

ClipKind clipKind(const QJsonObject &track, const QJsonObject &clip) {
    const QString kind = clip.value(QStringLiteral("kind")).toString();
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    const QString name = track.value(QStringLiteral("name")).toString().toLower();
    if (kind == QStringLiteral("caption") || type.contains(QStringLiteral("subtitle")) || type.contains(QStringLiteral("caption")))
        return ClipKind::Caption;
    if (kind == QStringLiteral("generator") || type.contains(QStringLiteral("title")) ||
        type.contains(QStringLiteral("graphic")) || type.contains(QStringLiteral("overlay")))
        return ClipKind::Generator;
    const bool music = type.contains(QStringLiteral("music")) || type.contains(QStringLiteral("score")) ||
                       name.contains(QStringLiteral("music")) || name.contains(QStringLiteral("score"));
    if (music) return ClipKind::Music;
    if (type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"))) return ClipKind::Audio;
    return ClipKind::Video;
}

ClipKind trackKind(const QJsonObject &track) {
    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    return clipKind(track, clips.isEmpty() ? QJsonObject() : clips.first().toObject());
}

const ClipStyle &EditorTheme::clip(ClipKind kind) const {
    switch (kind) {
    case ClipKind::Audio: return clipAudio;
    case ClipKind::Music: return clipMusic;
    case ClipKind::Generator: return clipGenerator;
    case ClipKind::Caption: return clipCaption;
    case ClipKind::Video: break;
    }
    return clipVideo;
}

QColor EditorTheme::marker(const QString &name) const {
    const QString key = name.trimmed().toLower();
    if (key == QStringLiteral("red")) return markerRed;
    if (key == QStringLiteral("orange")) return markerOrange;
    if (key == QStringLiteral("yellow")) return markerYellow;
    if (key == QStringLiteral("cyan")) return markerCyan;
    if (key == QStringLiteral("blue")) return markerBlue;
    if (key == QStringLiteral("purple")) return markerPurple;
    if (key == QStringLiteral("pink")) return markerPink;
    if (key == QStringLiteral("white")) return markerWhite;
    return markerGreen;
}

QColor kerfAccent(bool dark) { return dark ? QColor("#c39acb") : QColor("#6a3471"); }

EditorTheme makeEditorTheme(bool dark) {
    EditorTheme t;
    t.dark = dark;

    // Kerf's own accent: an aubergine ink on cream, its mauve by lamplight.
    // The mauve is light, so it carries dark ink, as dark-mode pills do.
    t.accent = tone(dark, "#6a3471", "#c39acb");
    t.accentTop = tone(dark, "#7a4381", "#cfa9d6");
    t.accentPressed = tone(dark, "#562a5c", "#a882b1");
    t.accentText = t.accent;
    t.accentTint = tone(dark, "#e2d6d3", "#382d32");
    t.accentTintPressed = tone(dark, "#d2c7c4", "#2a2226");
    t.onAccent = tone(dark, "#fbf6ec", "#21161f");
    t.onAccentSecondary = tone(dark, "#fbf6eca5", "#f4ecdea6");

    // Surfaces: book paper in light (the desk a step deeper than the page),
    // espresso in dark, never pure white or black.
    t.window = tone(dark, "#e6dece", "#15110e");
    t.chrome = tone(dark, "#eee7d8", "#241e19");
    t.panel = tone(dark, "#f4eee2", "#1c1713");
    t.panelTitle = tone(dark, "#e9e1d1", "#17130f");
    t.panelTitleActive = t.panel;
    t.raised = tone(dark, "#faf6ee", "#2a231d");
    t.group = tone(dark, "#ede6d7", "#231d18");
    t.groupBorder = tone(dark, "#ddd3c0", "#3a3029");
    t.well = tone(dark, "#f4eee2", "#19140f");
    t.dialog = tone(dark, "#f0e9db", "#1f1a15");

    // Lines: warm and low-contrast.
    t.dockSeparator = tone(dark, "#d6ccb9", "#3a3029");
    t.dockSeparatorHover = tone(dark, "#ab9f8b", "#5f5246");
    t.divider = tone(dark, "#dcd2bf", "#342b24");
    t.borderSubtle = tone(dark, "#e4dbcb", "#2e2620");
    t.border = tone(dark, "#d2c7b3", "#463b32");
    t.borderStrong = tone(dark, "#a99c87", "#66594c");

    // Text: warm near-black on cream, cream on espresso.
    t.text = tone(dark, "#2a231d", "#e6dccb");
    t.textStrong = tone(dark, "#1b1612", "#f4ecde");
    t.textSecondary = tone(dark, "#675b4f", "#a89a8a");
    t.placeholderText = tone(dark, "#6c6054", "#a09282");
    t.textDisabled = tone(dark, "#a89c8c", "#6d6156");
    t.tabInactiveText = tone(dark, "#62564a", "#a3958a");

    // Selection. In dark it is a quiet deep plum, so dense lists do not glow.
    t.selection = tone(dark, "#6a3471", "#583863");
    t.selectionText = tone(dark, "#fbf6ec", "#f4ecde");
    t.selectionInactive = tone(dark, "#e0d6c3", "#3a3029");
    t.selectionInactiveText = tone(dark, "#2a231d", "#f4ecde");
    t.textSelection = tone(dark, "#d8c9d2", "#675363");
    t.hover = tone(dark, "#4a341e0f", "#ffecd212");
    t.pressed = tone(dark, "#4a341e1c", "#ffecd220");

    t.field = tone(dark, "#fbf8f1", "#2a231d");
    t.fieldBorder = tone(dark, "#8b7e6d", "#7a6c5e");
    t.fieldDisabled = tone(dark, "#f1ebdf", "#221c17");
    t.focusRing = t.accent;
    t.focusHalo = t.accent;
    t.focusHalo.setAlpha(89); // 35%
    t.checkFill = tone(dark, "#fbf8f1", "#2a231d");
    t.checkBorder = tone(dark, "#8c7f6e", "#85776a");

    t.button = tone(dark, "#fbf8f1", "#3a3029");
    t.buttonEdge = tone(dark, "#3c28142e", "#3a3029");
    t.buttonTop = tone(dark, "#3c28141a", "#4a3f36");
    t.buttonBottom = tone(dark, "#3c28144c", "#2e2620");
    t.buttonHover = tone(dark, "#fdfbf6", "#43382f");
    t.buttonPressed = tone(dark, "#ebe3d4", "#4a3f36");
    t.buttonText = tone(dark, "#2a231d", "#f4ecde");
    t.buttonDisabled = tone(dark, "#f5f0e6", "#2a231d");
    t.buttonDisabledEdge = tone(dark, "#3c28141e", "#322a23");
    t.buttonDisabledText = tone(dark, "#a89c8c", "#7d7064");

    t.menu = tone(dark, "#faf6ee", "#2a231d");
    t.menuBorder = tone(dark, "#cdc1ac", "#463b32");
    t.menuText = tone(dark, "#2a231d", "#f4ecde");
    t.menuSelection = t.selection;
    t.menuSelectionText = t.selectionText;
    t.menuSeparator = tone(dark, "#e4dbcb", "#3a3029");
    t.tooltip = hex("#2f2721");
    t.tooltipBorder = tone(dark, "#2f2721", "#463b32");
    t.tooltipText = tone(dark, "#f3ebdd", "#f4ecde");
    t.scrollHandle = tone(dark, "#3c2a1a8e", "#ffecd26e");
    t.scrollHandleHover = tone(dark, "#3c2a1abe", "#ffecd2a0");

    t.tabText = tone(dark, "#62564a", "#a3958a");
    t.tabSelected = tone(dark, "#faf6ee", "#3a3029");
    t.tabSelectedBorder = tone(dark, "#d2c7b3", "#463b32");
    t.tabSelectedText = tone(dark, "#1b1612", "#f4ecde");
    t.sliderGroove = tone(dark, "#e2d9c8", "#3a3029");
    t.sliderGrooveBorder = tone(dark, "#d2c7b3", "#463b32");
    t.sliderHandle = tone(dark, "#fbf8f1", "#e6dccb");
    t.sliderHandleBorder = tone(dark, "#8c7f6e", "#8e8072");
    t.statusBar = tone(dark, "#eee7d8", "#241e19");
    t.statusText = tone(dark, "#62564a", "#a3958a");
    t.statusDivider = tone(dark, "#dcd2bf", "#3a3029");

    // Status: oxblood, sienna and moss, inks rather than signal lights.
    t.error = tone(dark, "#a3302b", "#e8877b");
    t.errorSurface = tone(dark, "#f5e2d9", "#3a221d");
    t.errorBorder = tone(dark, "#e2b8aa", "#6c3a31");
    t.errorText = tone(dark, "#86241f", "#f4bbb0");
    t.warning = tone(dark, "#9a4a0e", "#e2a062");
    t.warningSurface = tone(dark, "#f4e6c9", "#2e2620");
    t.warningBorder = tone(dark, "#e2cc9c", "#3e342c");
    t.warningText = tone(dark, "#2a231d", "#f4ecded2");
    t.warningTitle = tone(dark, "#1b1612", "#f4ecde");
    t.warningGlyph = tone(dark, "#9a4a0e", "#e59a4a");
    t.success = tone(dark, "#3d6a2e", "#93bf78");

    // Kerf's own surfaces: the chat is a page laid on the desk; its rows have
    // no boxes (their fills and edges are the page), its marks are the accent.
    t.assistant = t.accent;
    t.assistantSurface = tone(dark, "#e9dfd9", "#302729");
    t.assistantBorder = tone(dark, "#cbb6c0", "#584655");
    t.assistantText = tone(dark, "#5e2c65", "#d6b6dc");
    t.assistantHover = tone(dark, "#dacacd", "#42353c");
    t.chatSurface = tone(dark, "#f6f1e6", "#231d18");
    t.chatText = t.text;
    t.chatSecondary = t.textSecondary;
    t.chatChip = tone(dark, "#4a341e12", "#ffecd216");
    t.chatChipHover = tone(dark, "#4a341e1e", "#ffecd224");
    t.chatChipText = t.text;
    t.chatAvatarUser = tone(dark, "#e0d6c3", "#4a3f36");
    t.chatAvatarUserText = tone(dark, "#3d342c", "#efe6d6");
    t.chatAvatarKerf = t.accent;
    t.chatAvatarKerfText = t.onAccent;
    t.chatExpander = t.accentText;
    t.chatComposerRule = tone(dark, "#cdc1ac", "#4a3f36");
    t.chatComposerRuleFocus = t.accent;
    // The secondary ink at 35%, and 60% under the pointer.
    t.chatScrollHandle = tone(dark, "#675b4f59", "#a89a8a59");
    t.chatScrollHandleHover = tone(dark, "#675b4f99", "#a89a8a99");
    t.messageUser = t.chatSurface;
    t.messageUserBorder = t.messageUser;
    t.messageAgent = t.chatSurface;
    t.messageAgentBorder = t.messageAgent;
    t.messageText = t.chatText;
    t.messageUserRole = t.chatText;
    t.railButton = tone(dark, "#4a341e14", "#ffecd218");
    t.railButtonHover = tone(dark, "#4a341e22", "#ffecd226");
    t.railButtonActive = t.railButton;
    t.railGlyph = tone(dark, "#2a231d", "#efe6d6");
    t.railGlyphActive = t.accentText;
    t.railLabel = tone(dark, "#3d342c", "#ddd2c1");
    t.pillTonal = tone(dark, "#4a341e14", "#ffecd218");
    t.pillTonalText = t.text;

    t.glyph = tone(dark, "#3d342c", "#ddd2c1");
    t.glyphDisabled = tone(dark, "#b3a796", "#64584d");
    t.toolSelected = tone(dark, "#e2d9c8", "#3a3029");

    // Timeline. Track toggles that are on are a neutral ink plate, not the
    // accent, so purple stays on Kerf, the playhead, focus and the default action.
    t.timelineBackground = tone(dark, "#e6dece", "#15110e");
    t.ruler = tone(dark, "#f8f3ea", "#27211b");
    t.rulerBottom = tone(dark, "#efe8db", "#231d18");
    t.rulerTick = tone(dark, "#8a7d6c", "#75685b");
    t.rulerTickHalf = tone(dark, "#aa9e8c", "#5f5347");
    t.rulerTickMinor = tone(dark, "#c8bdab", "#4a3f36");
    t.rulerText = tone(dark, "#62564a", "#ab9e8e");
    t.rulerCorner = tone(dark, "#f4eee2", "#201a16");
    t.trackHeader = tone(dark, "#f1eadd", "#251f1a");
    t.trackHeaderEdge = tone(dark, "#faf6ee", "#2e2620");
    t.trackHeaderColumn = tone(dark, "#ebe3d4", "#1f1a15");
    t.trackHeaderText = tone(dark, "#2a231d", "#e6dccb");
    t.trackHeaderTextDisabled = tone(dark, "#a89c8c", "#75685b");
    t.trackHeaderDetail = tone(dark, "#62564a", "#a3958a");
    t.trackResizeGrip = tone(dark, "#8a7d6c", "#8e8072");
    t.trackBadgeText = tone(dark, "#675b4f", "#a89a8a");
    t.trackToggleOn = tone(dark, "#675b4f", "#a89a8a");
    t.trackToggle = tone(dark, "#675b4f66", "#a89a8a66"); // The same ink at 40%.
    t.trackToggleHover = tone(dark, "#4a341e17", "#ffecd21c");
    t.trackLane = tone(dark, "#efe8db", "#201a16");
    t.trackLaneAlt = tone(dark, "#ebe4d6", "#231d18");
    t.trackLaneLocked = tone(dark, "#4a341e16", "#ffecd20e");
    t.trackDivider = tone(dark, "#ddd3c1", "#14100d");
    t.trackBankDivider = tone(dark, "#b9ad98", "#0c0907");
    t.headerPreview = tone(dark, "#e2d9c8", "#2a231d");
    t.headerPreviewBorder = tone(dark, "#c8bdab", "#14100d");
    t.addLane = tone(dark, "#ebe4d6", "#201a16");
    t.addLaneButton = tone(dark, "#fbf8f1", "#3a3029");
    t.addLaneButtonBorder = tone(dark, "#c8bdab", "#463b32");
    t.addLaneText = tone(dark, "#2a231d", "#f4ecde");

    // Clips: muted inks from an old atlas (slate, verdigris, olive, ochre,
    // umber), each label on its header at 5.5:1 or more. The selected ring is
    // cream over ink, not a second loud hue.
    // Picture clips are a greyed heather: Kerf's purple family, far duller
    // than its accent, and well away from blue.
    t.clipVideo = dark ? clipStyle("#2c2830","#4e4459","#2f2937","#f4ecde","#a39aa8") : clipStyle("#c9c3c6","#5c5068","#43394d","#fbf6ec","#463c50");
    t.clipAudio = dark ? clipStyle("#252e27","#325b50","#1c322c","#f4ecde","#8ea094") : clipStyle("#c4cabd","#3b6a5d","#2a4c43","#fbf6ec","#2c5046");
    t.clipMusic = dark ? clipStyle("#2d2c1b","#4b5728","#293016","#f4ecde","#9c9e7e") : clipStyle("#cac8b0","#566428","#3e481d","#fbf6ec","#404b1e");
    t.clipGenerator = dark ? clipStyle("#3b2b1a","#7a5222","#432d13","#f4ecde","#b69b7a") : clipStyle("#d6c6ad","#875a1d","#614115","#fbf6ec","#654416");
    t.clipCaption = dark ? clipStyle("#322b25","#5c5248","#332d28","#f4ecde","#a59b8f") : clipStyle("#cec7ba","#675d53","#4a433c","#fbf6ec","#4d463e");
    t.clipSelectedRing = tone(dark, "#fffbf3", "#fbf4e8");
    t.clipSelectedRingOuter = tone(dark, "#1b1612", "#0c0907");
    t.clipTrim = tone(dark, "#1b1612", "#fbf4e8");
    t.clipHighlight = tone(dark, "#fffaf046", "#fff0dc22");
    t.clipFxBadge = tone(dark, "#140c0660", "#0000006e");
    t.clipFxBadgeText = tone(dark, "#fbf6ec", "#f4ecde");
    t.filmstripSeam = tone(dark, "#140c065a", "#00000078");
    t.waveCenter = tone(dark, "#140c0628", "#00000046");
    t.playhead = t.accent;
    t.playheadHalo = tone(dark, "#fbf8f1", "#0c0907");
    t.playheadIdle = tone(dark, "#8a7d6c", "#766a5d");
    t.skimmer = tone(dark, "#2a231dbe", "#f4ecded7");
    t.skimmerText = tone(dark, "#2a231d", "#f4ecde");
    t.scrubTrack = tone(dark, "#e2d9c8", "#3a3029");
    t.inOutRange = tone(dark, "#885d87", "#a180a2");
    t.snapLine = tone(dark, "#1b1612be", "#fff6e8dc");
    t.readout = tone(dark, "#faf6eef5", "#241e19f2");
    t.readoutBorder = tone(dark, "#cdc1ac", "#463b32");
    t.readoutText = tone(dark, "#2a231d", "#f4ecde");
    t.dragShadow = hex("#140c0659");

    t.markerRed = tone(dark, "#b23a2e", "#e0675a");
    t.markerOrange = tone(dark, "#b4561a", "#e0904a");
    t.markerYellow = tone(dark, "#8a6d0c", "#d8bb58");
    t.markerGreen = tone(dark, "#4b7a34", "#86b06a");
    t.markerCyan = tone(dark, "#2f7a7c", "#6db3ad");
    t.markerBlue = tone(dark, "#3e5f8f", "#7f9cc6");
    t.markerPurple = tone(dark, "#7a3f86", "#b98bc4");
    t.markerPink = tone(dark, "#a8406e", "#d986a8");
    t.markerWhite = tone(dark, "#7f7365", "#e6dccb");

    // Proposals are drawn in Kerf's ink.
    t.proposalAdd = t.accent;
    t.proposalAddFill = tone(dark, "#6a347126", "#c39acb2e");
    t.proposalChange = tone(dark, "#5a2c60", "#ceadd5");
    t.proposalChangeFill = tone(dark, "#5a2c601e", "#ceadd524");
    t.proposalRemove = tone(dark, "#a3302b", "#e8877b");
    t.proposalRemoveHatch = tone(dark, "#a3302b6e", "#e8877b96");
    t.proposalUnderlay = tone(dark, "#fbf8f1d2", "#0c090796");

    // Viewers: the stage follows the appearance, as a video site's page does;
    // the picture card is warm black in both.
    t.viewerSurround = tone(dark, "#e6dece", "#120e0b");
    t.viewerCanvas = hex("#0a0807");
    t.viewerText = hex("#ab9e8e");
    t.viewerTextDim = hex("#958877");
    t.viewerCaption = hex("#0e0b09e1");
    t.viewerCaptionBorder = t.viewerCaption; // No box line around the caption.
    t.viewerCaptionText = hex("#f4ecde");
    t.deviceFrame = hex("#1e1d1f");
    t.deviceEdge = hex("#3a383c");
    t.deviceShadow = hex("#0000002e");
    t.deviceScreen = hex("#000000");
    t.deviceDeck = hex("#2b2a2d");
    t.deviceHinge = hex("#0e0d0f");
    t.deviceInk = hex("#ffffff");
    t.safeZoneFill = hex("#ffffff1f");
    t.safeZoneLine = hex("#ffffffa6");
    t.safeZoneText = hex("#ffffff");
    t.safeZoneLabel = hex("#000000a0");
    t.playerInk = hex("#ffffffe6");
    t.playerInkDisabled = hex("#ffffff66");
    t.playerShade = hex("#00000099");
    t.playerTopShade = hex("#00000080");
    t.playerHover = hex("#ffffff1f");
    t.playerTrack = hex("#ffffff4d");
    // The bar always lies on the picture's dark gradient, so it takes the
    // accent the dark appearance uses, in either.
    t.playerPlayed = kerfAccent(true);
    t.playerTipFill = hex("#000000b3");
    t.maskOutline = hex("#fffbf3");
    t.maskOutlineIdle = hex("#fffbf3b3");
    t.maskShadow = hex("#0000008c");
    t.maskFill = hex("#fffbf31e");
    t.maskHandleFill = t.accent;
    t.maskHandleBorder = hex("#fffbf3");
    t.maskTag = hex("#00000099");
    t.maskTagSelected = t.selection;
    t.maskText = hex("#fffbf3");

    t.thumbnail = tone(dark, "#2a231d", "#0e0b09");
    t.tileEmpty = tone(dark, "#e2d9c8", "#2a231d");
    t.tileEmptyBorder = tone(dark, "#e2d9c8", "#3a3029");
    t.tileEmptyText = tone(dark, "#5a4e43", "#a89a8a");
    t.audioTile = t.clipAudio.fill;
    t.audioTileGlyph = t.clipAudio.wave;
    t.cardPlaceholder = tone(dark, "#ebe4d6", "#2a231d");
    t.cardPlaceholderText = tone(dark, "#5a4e43", "#a89a8a");
    t.badgeError = hex("#a3302b");
    t.badgeErrorText = hex("#fbf6ec");
    t.badgeInfo = hex("#6a3471");
    t.badgeInfoText = hex("#fbf6ec");
    t.badgeNeutral = tone(dark, "#140c06a0", "#000000a0");
    t.badgeNeutralText = tone(dark, "#fbf6ec", "#f4ecde");

    // Guards on the palette above: each holds already, and keeps holding if a
    // value is retuned. Text and glyphs on the filled accent read at 4.5:1
    // (no exemption)...
    t.onAccent = readableOn(t.onAccent, t.accent, 4.5);
    t.onAccentSecondary = readableAlphaOn(t.onAccentSecondary, t.selection, 4.5);
    t.selectionText = readableOn(t.selectionText, t.selection, 4.5);
    t.menuSelectionText = readableOn(t.menuSelectionText, t.menuSelection, 4.5);
    t.trackBadgeText = readableOn(t.trackBadgeText, t.trackHeader, 4.5);
    // ...the accent as text reads on every surface it is drawn on (the
    // transports' timecodes, the timeline's corner time, the sync chip,
    // cards, fields and dialogs), all light or all dark, so reading on one
    // never costs another...
    for (const QColor &surface : {t.panel, t.rulerCorner, t.statusBar, t.raised, t.window, t.chrome, t.well, t.field,
                                  t.dialog, t.group})
        t.accentText = readableOn(t.accentText, surface, 4.5);
    t.railGlyphActive = t.accentText;
    t.chatExpander = t.accentText;
    // ...the marked range is information, 3:1 on the panel around the bar...
    t.inOutRange = readableOn(t.inOutRange, t.panel, 3.0);
    // ...the playhead's line always has its halo either side, so it is read
    // against the halo wherever it crosses (a picture, a clip, a bare lane),
    // and its head against the ruler from top to foot. The skimmed playhead
    // reads at 3:1 on each, no more than it must; the playhead clearly more,
    // so it stays the louder of the two...
    for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
        t.playheadIdle = justReadableOn(t.playheadIdle, surface, 3.0);
    for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
        t.playhead = justReadableOn(t.playhead, surface, std::max(3.0, contrastRatio(t.playheadIdle, surface) + 0.4));
    // ...every marker reads at 3:1 on the whole ruler...
    for (QColor *marker : {&t.markerRed, &t.markerOrange, &t.markerYellow, &t.markerGreen, &t.markerCyan, &t.markerBlue,
                           &t.markerPurple, &t.markerPink, &t.markerWhite})
        for (const QColor &surface : {t.ruler, t.rulerBottom}) *marker = readableOn(*marker, surface, 3.0);
    // ...and every clip's name reads on its header at 4.5:1.
    for (ClipStyle *style : {&t.clipVideo, &t.clipAudio, &t.clipMusic, &t.clipGenerator, &t.clipCaption})
        style->label = readableOn(style->label, style->header, 4.5);
    // The Inspector's controls: aliases (the launcher's switch and knob).
    t.inspectorField = dark ? t.field : t.group;
    t.switchOn = t.trackToggleOn;
    t.switchOnKnob = tone(dark, "#fffcf6", "#21161f");
    t.switchOff = tone(dark, "#e2d9c8", "#3a3029");
    t.switchOffStroke = tone(dark, "#8b7e6d", "#85776a");
    t.knob = tone(dark, "#fffcf6", "#efe6d6");
    t.inspectorDetent = t.rulerTickHalf;
    t.sliderHandleChanged = dark ? t.borderStrong : t.text;
    return t;
}

QPalette editorPalette(const EditorTheme &t) {
    QPalette palette;
    for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive}) {
        palette.setColor(group, QPalette::WindowText, t.text);
        palette.setColor(group, QPalette::Base, t.field);
        palette.setColor(group, QPalette::AlternateBase, t.well);
        palette.setColor(group, QPalette::Text, t.text);
        palette.setColor(group, QPalette::Button, t.button.alpha() == 255 ? t.button : t.raised);
        palette.setColor(group, QPalette::ButtonText, t.buttonText);
        palette.setColor(group, QPalette::BrightText, t.textStrong);
        palette.setColor(group, QPalette::ToolTipBase, t.tooltip);
        palette.setColor(group, QPalette::ToolTipText, t.tooltipText);
        palette.setColor(group, QPalette::PlaceholderText, t.placeholderText);
        palette.setColor(group, QPalette::Link, t.accentText);
        palette.setColor(group, QPalette::Light, t.dark ? t.borderStrong : QColor("#ffffff"));
        palette.setColor(group, QPalette::Midlight, t.border);
        palette.setColor(group, QPalette::Mid, t.borderStrong);
        palette.setColor(group, QPalette::Dark, t.dark ? t.divider : t.borderStrong);
    }
    palette.setColor(QPalette::Active, QPalette::Highlight, t.selection);
    palette.setColor(QPalette::Active, QPalette::HighlightedText, t.selectionText);
    palette.setColor(QPalette::Inactive, QPalette::Highlight, t.selectionInactive);
    palette.setColor(QPalette::Inactive, QPalette::HighlightedText, t.selectionInactiveText);
    for (const QPalette::ColorRole role : {QPalette::WindowText, QPalette::Text, QPalette::ButtonText})
        palette.setColor(QPalette::Disabled, role, t.textDisabled);
    palette.setColor(QPalette::Disabled, QPalette::Base, t.fieldDisabled);
    palette.setColor(QPalette::Disabled, QPalette::Button, t.buttonDisabled.alpha() == 255 ? t.buttonDisabled : t.raised);
    palette.setColor(QPalette::Disabled, QPalette::Highlight, t.selectionInactive);
    palette.setColor(QPalette::Disabled, QPalette::HighlightedText, t.textDisabled);
    return palette;
}

ColorScheme editorColorScheme() { return state().scheme; }

void setEditorColorScheme(ColorScheme scheme) {
    AppearanceState &s = state();
    s.scheme = scheme;
    s.computed = false;
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    // Native parts (title bars, file and colour dialogs) follow a forced
    // appearance too; System hands the choice back to the platform.
    if (qGuiApp) {
        QGuiApplication::styleHints()->setColorScheme(scheme == ColorScheme::Dark ? Qt::ColorScheme::Dark
                                                      : scheme == ColorScheme::Light ? Qt::ColorScheme::Light
                                                                                     : Qt::ColorScheme::Unknown);
    }
#endif
    if (qApp) applyEditorAppearance();
}

const EditorTheme &editorTheme() {
    AppearanceState &s = state();
    if (!s.computed) {
        const bool dark = schemeIsDark(s.scheme);
        s.theme = makeEditorTheme(dark);
        s.computed = true;
    }
    return s.theme;
}

void applyEditorAppearance() {
    AppearanceState &s = state();
    if (s.applying || !qApp) return;
    const bool dark = schemeIsDark(s.scheme);
    if (s.applied && s.computed && s.theme.dark == dark) return;
    s.applying = true;
    s.theme = makeEditorTheme(dark);
    s.computed = true;
    s.applied = true;
    QApplication::setPalette(editorPalette(s.theme));
    qApp->setStyleSheet(editorStyleSheet(s.theme));
    // The style sheet repolishes every widget; painted ones read the theme
    // as they paint.
    for (QWidget *widget : QApplication::allWidgets()) widget->update();
    emit EditorAppearance::instance()->changed();
    s.applying = false;
}

void followSystemAppearance() {
    static PaletteWatcher *watcher = nullptr;
    if (watcher || !qApp) return;
    watcher = new PaletteWatcher(qApp);
    qApp->installEventFilter(watcher);
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    QObject::connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged, watcher,
                     [] { applyEditorAppearance(); });
#endif
}

EditorAppearance *EditorAppearance::instance() {
    // Lives as long as the application, and goes with it.
    static QPointer<EditorAppearance> notifier;
    if (!notifier) {
        notifier = new EditorAppearance;
        notifier->setParent(QCoreApplication::instance());
    }
    return notifier;
}

} // namespace kerf
