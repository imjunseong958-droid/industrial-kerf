#include "editor_theme.h"
#include "editor_style.h"

#include <QApplication>
#include <QEvent>
#include <QGuiApplication>
#include <QDir>
#include <QHash>
#include <QImage>
#include <QPainter>
#include <QPolygonF>
#include <QPointer>
#include <QTemporaryDir>
#include <QStyleHints>
#include <QWidget>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

QColor rgba(int r, int g, int b, int a) { return QColor(r, g, b, a); }

// `color` moved towards white (on a dark `background`) or black (on a light
// one) until it reads at `ratio`.
QColor readableOn(const QColor &color, const QColor &background, qreal ratio) {
    const QColor target = background.lightness() < 128 ? QColor(Qt::white) : QColor(Qt::black);
    QColor result = color;
    for (int step = 1; step <= 20 && contrastRatio(result, background) < ratio; ++step)
        result = mixColor(color, target, step * 0.05);
    return result;
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

// Re-applies on palette changes, which is how an accent change arrives
// (and an appearance change where Qt has no colour scheme to report).
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

QColor systemAccent(bool dark) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
    const QString platform = QGuiApplication::platformName();
    if (platform != QStringLiteral("offscreen") && platform != QStringLiteral("minimal")) {
        const QColor accent = QGuiApplication::palette().color(QPalette::Accent);
        if (accent.isValid() && accent.alpha() == 255) return accent;
    }
#endif
    return dark ? QColor("#0a84ff") : QColor("#007aff");
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

EditorTheme makeEditorTheme(bool dark, const QColor &systemAccentColor) {
    EditorTheme t;
    t.dark = dark;
    const QColor accent = systemAccentColor.isValid() ? systemAccentColor : systemAccent(dark);
    t.accent = accent;
    t.accentTop = accent.lighter(112);
    t.accentPressed = accent.darker(dark ? 120 : 115);
    t.onAccent = QColor("#ffffff");
    t.onAccentSecondary = rgba(255, 255, 255, 204);
    // White text needs a deeper blue than the accent's own, as in macOS's
    // selected rows.
    t.selection = mixColor(accent, Qt::black, dark ? 0.16 : 0.12);
    t.selectionText = QColor("#ffffff");
    t.focusRing = accent;
    t.menuSelection = accent;
    t.menuSelectionText = QColor("#ffffff");
    t.trackToggleOn = accent;
    t.trackToggleOnGlyph = QColor("#ffffff");
    t.playhead = accent;
    t.maskOutline = accent.lighter(dark ? 115 : 125);
    t.maskOutlineIdle = t.maskOutline;
    t.maskOutlineIdle.setAlpha(170);
    t.maskFill = t.maskOutline;
    t.maskFill.setAlpha(35);
    t.onAssistant = QColor("#ffffff");

    if (dark) {
        t.window = QColor("#0e0e0e");
        t.chrome = QColor("#141414");
        t.panel = QColor("#1a1a1a");
        t.panelTitle = QColor("#1f1f1f");
        t.panelTitleActive = QColor("#262626");
        t.raised = QColor("#2a2a2a");
        t.well = QColor("#161616");
        t.dialog = QColor("#1c1c1c");

        t.dockSeparator = QColor("#0a0a0a");
        t.dockSeparatorHover = QColor("#4e4e4e");
        t.divider = QColor("#0a0a0a");
        t.borderSubtle = QColor("#2e2e2e");
        t.border = QColor("#404040");
        t.borderStrong = QColor("#5a5a5a");

        t.text = QColor("#d7d7d7");
        t.textStrong = QColor("#f2f2f2");
        t.textSecondary = QColor("#a5a5a5");
        t.textTertiary = QColor("#7c7c7c");
        t.textDisabled = QColor("#727272");

        t.selectionInactive = QColor("#464646");
        t.selectionInactiveText = QColor("#f2f2f2");
        t.hover = rgba(255, 255, 255, 20);
        t.pressed = rgba(255, 255, 255, 34);

        t.field = QColor("#2a2a2a");
        t.fieldBorder = QColor("#444444");
        t.fieldDisabled = QColor("#232323");

        t.button = QColor("#3d3d40");
        t.buttonEdge = QColor("#48484b");
        t.buttonTop = QColor("#545457");
        t.buttonBottom = QColor("#333336");
        t.buttonHover = QColor("#47474a");
        t.buttonPressed = QColor("#535356");
        t.buttonText = QColor("#e6e6e6");
        t.buttonDisabled = QColor("#262628");
        t.buttonDisabledEdge = QColor("#2e2e30");
        t.buttonDisabledText = QColor("#6e6e73");

        t.menu = QColor("#242426");
        t.menuBorder = QColor("#3a3a3c");
        t.menuText = QColor("#e6e6e6");
        t.menuSeparator = QColor("#3a3a3c");

        t.tooltip = QColor("#363636");
        t.tooltipBorder = QColor("#4d4d4d");
        t.tooltipText = QColor("#e6e6e6");

        t.scrollTrack = QColor("#101010");
        t.scrollHandle = QColor("#4f4f4f");
        t.scrollHandleHover = QColor("#6d6d6d");

        t.tabText = QColor("#999999");
        t.tabSelected = QColor("#2d2d2d");
        t.tabSelectedBorder = QColor("#424242");
        t.tabSelectedText = QColor("#f1f1f1");

        t.sliderGroove = QColor("#101010");
        t.sliderGrooveBorder = QColor("#2c2c2c");
        t.sliderHandle = QColor("#b3b3b3");
        t.sliderHandleBorder = QColor("#d6d6d6");

        t.statusBar = QColor("#141414");
        t.statusText = QColor("#8e8e8e");
        t.statusDivider = QColor("#262626");

        t.error = QColor("#ff6961");
        t.errorSurface = QColor("#3a2426");
        t.errorBorder = QColor("#7d4045");
        t.errorText = QColor("#ffb4ae");
        t.warning = QColor("#ff9f5a");
        t.warningSurface = QColor("#3a2d23");
        t.warningBorder = QColor("#7a5235");
        t.warningText = QColor("#ffc59c");
        t.success = QColor("#30d158");

        t.assistant = QColor("#bf5af2");
        t.assistantStrong = QColor("#9b40d1");
        t.assistantSurface = QColor("#34243d");
        t.assistantBorder = QColor("#64377b");
        t.assistantText = QColor("#dfacf8");

        t.messageUser = QColor("#333333");
        t.messageUserBorder = QColor("#494949");
        t.messageAgent = QColor("#262626");
        t.messageAgentBorder = QColor("#353535");
        t.messageText = QColor("#e4e4e4");
        t.messageUserRole = QColor("#bfbfbf");

        t.glyph = QColor("#d7d7d7");
        t.glyphDisabled = QColor("#727272");

        t.timelineBackground = QColor("#121212");
        t.ruler = QColor("#2a2a2a");
        t.rulerBottom = QColor("#202020");
        t.rulerTick = QColor("#666666");
        t.rulerTickHalf = QColor("#5a5a5a");
        t.rulerTickMinor = QColor("#474747");
        t.rulerText = QColor("#b0b0b0");
        t.rulerCorner = QColor("#1a1a1a");
        t.trackHeader = QColor("#1f1f1f");
        t.trackHeaderEdge = QColor("#282828");
        t.trackHeaderColumn = QColor("#171717");
        t.trackHeaderText = QColor("#d8d8d8");
        t.trackHeaderTextDisabled = QColor("#808080");
        t.trackHeaderDetail = QColor("#7c7c7c");
        t.trackResizeGrip = QColor("#8a8a8a");
        t.trackBadge = QColor("#2a2a2a");
        t.trackBadgeBorder = QColor("#454545");
        t.trackBadgeText = QColor("#ececec");
        t.trackToggle = QColor("#2a2a2a");
        t.trackToggleBorder = QColor("#454545");
        t.trackToggleGlyph = QColor("#9a9a9a");
        t.trackLane = QColor("#1c1c1c");
        t.trackLaneAlt = QColor("#1f1f1f");
        t.trackLaneLocked = rgba(255, 255, 255, 14);
        t.trackDivider = QColor("#0c0c0c");
        t.trackBankDivider = QColor("#050505");
        t.headerPreview = QColor("#0e0e0e");
        t.headerPreviewBorder = QColor("#0a0a0a");
        t.addLane = QColor("#1d1d1d");
        t.addLaneButton = QColor("#2a2a2a");
        t.addLaneButtonBorder = QColor("#555555");
        t.addLaneText = QColor("#ededed");

        t.clipVideo = clipStyle("#1b2a3c", "#325985", "#0b1622", "#f5f5f7", "#95bae4");
        t.clipAudio = clipStyle("#17362f", "#246b5d", "#0a1f1b", "#f5f5f7", "#79d8c5");
        t.clipMusic = clipStyle("#1a321d", "#2f6a37", "#0c1d0e", "#f5f5f7", "#81cf8c");
        t.clipGenerator = clipStyle("#391d2c", "#863761", "#210d18", "#f5f5f7", "#e9a5c9");
        t.clipCaption = clipStyle("#2f2d18", "#69632b", "#1d1b0c", "#f5f5f7", "#e0d785");
        t.clipSelectedRing = QColor("#ffffff");
        t.clipTrim = QColor("#ffffff");
        t.clipHighlight = rgba(255, 255, 255, 40);
        t.clipFxBadge = rgba(0, 0, 0, 110);
        t.clipFxBadgeText = QColor("#ededed");
        t.filmstripSeam = rgba(0, 0, 0, 120);
        t.waveCenter = rgba(0, 0, 0, 70);
        t.playheadHalo = rgba(0, 0, 0, 110);
        t.playheadIdle = QColor("#6a6a6a");
        t.skimmer = rgba(240, 240, 240, 215);
        t.skimmerText = QColor("#f4f4f4");
        t.scrubTrack = QColor("#262626");
        t.snapLine = rgba(255, 255, 255, 220);
        t.readout = rgba(10, 10, 10, 235);
        t.readoutBorder = QColor("#4a4a4a");
        t.readoutText = QColor("#f2f2f2");

        t.markerRed = QColor("#e5534b");
        t.markerOrange = QColor("#e8913a");
        t.markerYellow = QColor("#e6c84a");
        t.markerGreen = QColor("#5fb85f");
        t.markerCyan = QColor("#4ec4d0");
        t.markerBlue = QColor("#4f8de0");
        t.markerPurple = QColor("#a275e0");
        t.markerPink = QColor("#e078b4");
        t.markerWhite = QColor("#e8e8e8");

        t.proposalRemove = QColor("#e86060");
        t.proposalRemoveHatch = rgba(232, 96, 96, 150);

        t.thumbnail = QColor("#0e0e0e");
        t.thumbnailText = QColor("#6f6f6f");
        t.cardPlaceholder = QColor("#2a2a2e");
        t.cardPlaceholderText = QColor("#8a8a92");
        t.badgeError = QColor("#5a262c");
        t.badgeErrorText = QColor("#f3b9be");
        t.badgeNeutral = QColor("#2e2e2e");
        t.badgeNeutralText = QColor("#cfcfcf");
    } else {
        t.window = QColor("#ececec");
        t.chrome = QColor("#f5f5f5");
        t.panel = QColor("#f6f6f6");
        t.panelTitle = QColor("#e8e8ea");
        t.panelTitleActive = QColor("#f6f6f6");
        t.raised = QColor("#ffffff");
        t.well = QColor("#ffffff");
        t.dialog = QColor("#ececec");

        t.dockSeparator = QColor("#d5d5d8");
        t.dockSeparatorHover = QColor("#aeaeb2");
        t.divider = QColor("#d5d5d8");
        t.borderSubtle = QColor("#e3e3e6");
        t.border = QColor("#d5d5d8");
        t.borderStrong = QColor("#b4b4b9");

        t.text = QColor("#1d1d1f");
        t.textStrong = QColor("#000000");
        t.textSecondary = QColor("#6e6e73");
        t.textTertiary = QColor("#8e8e93");
        t.textDisabled = QColor("#a5a5aa");

        t.selectionInactive = QColor("#dcdcdf");
        t.selectionInactiveText = QColor("#1d1d1f");
        t.hover = rgba(0, 0, 0, 13);
        t.pressed = rgba(0, 0, 0, 26);

        t.field = QColor("#ffffff");
        t.fieldBorder = QColor("#c9c9ce");
        t.fieldDisabled = QColor("#f5f5f7");

        t.button = QColor("#ffffff");
        t.buttonEdge = rgba(0, 0, 0, 36);
        t.buttonTop = rgba(0, 0, 0, 28);
        t.buttonBottom = rgba(0, 0, 0, 72);
        t.buttonHover = QColor("#fafafa");
        t.buttonPressed = QColor("#ebebed");
        t.buttonText = QColor("#1d1d1f");
        t.buttonDisabled = QColor("#fafafa");
        t.buttonDisabledEdge = rgba(0, 0, 0, 24);
        t.buttonDisabledText = QColor("#a5a5aa");

        t.menu = QColor("#ffffff");
        t.menuBorder = QColor("#c9c9ce");
        t.menuText = QColor("#1d1d1f");
        t.menuSeparator = QColor("#e3e3e6");

        t.tooltip = QColor("#f5f5f7");
        t.tooltipBorder = QColor("#c9c9ce");
        t.tooltipText = QColor("#1d1d1f");

        t.scrollTrack = QColor("#f0f0f2");
        t.scrollHandle = QColor("#c1c1c5");
        t.scrollHandleHover = QColor("#a1a1a6");

        t.tabText = QColor("#6e6e73");
        t.tabSelected = QColor("#ffffff");
        t.tabSelectedBorder = QColor("#d1d1d6");
        t.tabSelectedText = QColor("#1d1d1f");

        t.sliderGroove = QColor("#e3e3e6");
        t.sliderGrooveBorder = QColor("#d1d1d6");
        t.sliderHandle = QColor("#ffffff");
        t.sliderHandleBorder = QColor("#b4b4b9");

        t.statusBar = QColor("#f5f5f5");
        t.statusText = QColor("#6e6e73");
        t.statusDivider = QColor("#e0e0e3");

        t.error = QColor("#d70015");
        t.errorSurface = QColor("#fdecee");
        t.errorBorder = QColor("#f3b7bd");
        t.errorText = QColor("#a1000f");
        t.warning = QColor("#c93400");
        t.warningSurface = QColor("#faede8");
        t.warningBorder = QColor("#eab29e");
        t.warningText = QColor("#b52f00");
        t.success = QColor("#1e7b34");

        t.assistant = QColor("#af52de");
        t.assistantStrong = QColor("#9440c2");
        t.assistantSurface = QColor("#f7eefc");
        t.assistantBorder = QColor("#ddb6f1");
        t.assistantText = QColor("#7a399b");

        t.messageUser = QColor("#ececee");
        t.messageUserBorder = QColor("#dcdcdf");
        t.messageAgent = QColor("#ffffff");
        t.messageAgentBorder = QColor("#e3e3e6");
        t.messageText = QColor("#1d1d1f");
        t.messageUserRole = QColor("#6e6e73");

        t.glyph = QColor("#3a3a3c");
        t.glyphDisabled = QColor("#a5a5aa");

        t.timelineBackground = QColor("#e6e6e8");
        t.ruler = QColor("#fbfbfb");
        t.rulerBottom = QColor("#efeff1");
        t.rulerTick = QColor("#8e8e93");
        t.rulerTickHalf = QColor("#aeaeb2");
        t.rulerTickMinor = QColor("#c7c7cc");
        t.rulerText = QColor("#6e6e73");
        t.rulerCorner = QColor("#f6f6f6");
        t.trackHeader = QColor("#f6f6f6");
        t.trackHeaderEdge = QColor("#ffffff");
        t.trackHeaderColumn = QColor("#efeff1");
        t.trackHeaderText = QColor("#1d1d1f");
        t.trackHeaderTextDisabled = QColor("#a5a5aa");
        t.trackHeaderDetail = QColor("#8e8e93");
        t.trackResizeGrip = QColor("#8e8e93");
        t.trackBadge = QColor("#ffffff");
        t.trackBadgeBorder = QColor("#c9c9ce");
        t.trackBadgeText = QColor("#1d1d1f");
        t.trackToggle = QColor("#ffffff");
        t.trackToggleBorder = QColor("#c9c9ce");
        t.trackToggleGlyph = QColor("#6e6e73");
        t.trackLane = QColor("#f4f4f6");
        t.trackLaneAlt = QColor("#efeff1");
        t.trackLaneLocked = rgba(0, 0, 0, 20);
        t.trackDivider = QColor("#dcdcdf");
        t.trackBankDivider = QColor("#c2c2c7");
        t.headerPreview = QColor("#e3e3e6");
        t.headerPreviewBorder = QColor("#c9c9ce");
        t.addLane = QColor("#efeff1");
        t.addLaneButton = QColor("#ffffff");
        t.addLaneButtonBorder = QColor("#c9c9ce");
        t.addLaneText = QColor("#1d1d1f");

        t.clipVideo = clipStyle("#cbdff6", "#2b6cb6", "#1f4e84", "#ffffff", "#235895");
        t.clipAudio = clipStyle("#c5ede5", "#1c7865", "#145749", "#ffffff", "#186d5c");
        t.clipMusic = clipStyle("#c9e9cd", "#2d7637", "#205527", "#ffffff", "#246b2d");
        t.clipGenerator = clipStyle("#f5d1e4", "#ae2970", "#841f55", "#ffffff", "#992462");
        t.clipCaption = clipStyle("#f8f3c8", "#e6d63f", "#a3961d", "#1d1d1f", "#807509");
        t.clipSelectedRing = QColor("#1d1d1f");
        t.clipTrim = QColor("#1d1d1f");
        t.clipHighlight = rgba(255, 255, 255, 90);
        t.clipFxBadge = rgba(0, 0, 0, 90);
        t.clipFxBadgeText = QColor("#ffffff");
        t.filmstripSeam = rgba(0, 0, 0, 90);
        t.waveCenter = rgba(0, 0, 0, 40);
        t.playheadHalo = rgba(255, 255, 255, 170);
        t.playheadIdle = QColor("#aeaeb2");
        t.skimmer = rgba(29, 29, 31, 190);
        t.skimmerText = QColor("#1d1d1f");
        t.scrubTrack = QColor("#e3e3e6");
        t.snapLine = rgba(0, 0, 0, 190);
        t.readout = rgba(255, 255, 255, 240);
        t.readoutBorder = QColor("#c9c9ce");
        t.readoutText = QColor("#1d1d1f");

        t.markerRed = QColor("#e0443a");
        t.markerOrange = QColor("#ea6f1a");
        t.markerYellow = QColor("#d4bf00");
        t.markerGreen = QColor("#2ea043");
        t.markerCyan = QColor("#1aa3b8");
        t.markerBlue = QColor("#2f76d9");
        t.markerPurple = QColor("#9b51e0");
        t.markerPink = QColor("#e0408a");
        t.markerWhite = QColor("#aeaeb2");

        t.proposalRemove = QColor("#d70015");
        t.proposalRemoveHatch = rgba(215, 0, 21, 110);

        t.thumbnail = QColor("#e3e3e6");
        t.thumbnailText = QColor("#8e8e93");
        t.cardPlaceholder = QColor("#ececee");
        t.cardPlaceholderText = QColor("#8e8e93");
        t.badgeError = QColor("#d70015");
        t.badgeErrorText = QColor("#ffffff");
        t.badgeNeutral = rgba(0, 0, 0, 150);
        t.badgeNeutralText = QColor("#ffffff");
    }

    // Derived from the accent and the surfaces above.
    t.accentText = readableOn(accent, t.panel, 4.5);
    t.accentTint = mixColor(t.panel, accent, dark ? 0.26 : 0.16);
    t.textSelection = mixColor(t.field, accent, dark ? 0.45 : 0.28);
    t.inOutRange = mixColor(t.scrubTrack, accent, dark ? 0.45 : 0.35);
    t.badgeInfo = dark ? mixColor(QColor("#141414"), accent, 0.4) : accent;
    t.badgeInfoText = dark ? readableOn(accent.lighter(130), t.badgeInfo, 4.5) : QColor("#ffffff");
    t.proposalAdd = t.assistant;
    t.proposalAddFill = t.assistant;
    t.proposalAddFill.setAlpha(dark ? 46 : 38);
    t.proposalChange = dark ? t.assistant.lighter(115) : t.assistant.darker(115);
    t.proposalChangeFill = t.proposalChange;
    t.proposalChangeFill.setAlpha(dark ? 36 : 30);
    t.audioTile = t.clipAudio.fill;
    t.audioTileGlyph = t.clipAudio.wave;

    // The viewer's field is the same in both appearances.
    t.viewerSurround = QColor("#1c1c1c");
    t.viewerCanvas = QColor("#050505");
    t.viewerText = QColor("#a0a0a0");
    t.viewerTextDim = QColor("#747474");
    t.viewerGlyph = QColor("#b1b1b1");
    t.viewerGlyphRing = QColor("#626262");
    t.viewerGlyphFill = QColor("#181818");
    t.viewerCaption = rgba(15, 15, 15, 225);
    t.viewerCaptionBorder = QColor("#676767");
    t.viewerCaptionText = QColor("#eeeeee");
    t.maskText = QColor("#ffffff");
    t.maskHandleFill = QColor("#141414");
    t.maskShadow = rgba(0, 0, 0, 140);
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
        palette.setColor(group, QPalette::PlaceholderText, t.textTertiary);
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

QString checkMarkImagePath() {
    static QTemporaryDir directory;
    static QString path;
    if (!path.isEmpty() || !directory.isValid()) return path;
    // check.png and check@2x.png: the style sheet names the first, and Qt
    // picks the second on Retina screens.
    for (const int scale : {1, 2}) {
        QImage image(14 * scale, 14 * scale, QImage::Format_ARGB32_Premultiplied);
        image.fill(Qt::transparent);
        QPainter painter(&image);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.scale(scale, scale);
        painter.setPen(QPen(Qt::white, 1.8, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawPolyline(QPolygonF{{3.6, 7.3}, {6.0, 9.7}, {10.6, 4.6}});
        painter.end();
        const QString file = directory.filePath(scale == 1 ? QStringLiteral("check.png") : QStringLiteral("check@2x.png"));
        if (!image.save(file)) return path;
    }
    path = QDir::fromNativeSeparators(directory.filePath(QStringLiteral("check.png")));
    return path;
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
        s.theme = makeEditorTheme(dark, systemAccent(dark));
        s.computed = true;
    }
    return s.theme;
}

void applyEditorAppearance() {
    AppearanceState &s = state();
    if (s.applying || !qApp) return;
    const bool dark = schemeIsDark(s.scheme);
    const QColor accent = systemAccent(dark);
    if (s.applied && s.computed && s.theme.dark == dark && s.theme.accent == accent) return;
    s.applying = true;
    s.theme = makeEditorTheme(dark, accent);
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
