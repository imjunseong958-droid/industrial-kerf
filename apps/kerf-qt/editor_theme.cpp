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
    t.focusHalo = accent;
    t.focusHalo.setAlphaF(0.35);
    t.menuSelection = accent;
    t.menuSelectionText = QColor("#ffffff");
    t.trackToggleOn = accent;
    t.trackToggleOnGlyph = QColor("#ffffff");
    t.playhead = accent;

    if (dark) {
        // The launcher's dark family: #1e1e1e content, #2c2c2e chrome, and
        // #3a3a3c hairlines.
        t.window = QColor("#1c1c1e");
        t.chrome = QColor("#2c2c2e");
        t.panel = QColor("#1e1e1e");
        // Inactive tabs sit back on a deeper strip; the current tab is the
        // lighter one and merges into its panel's body, as in light.
        t.panelTitle = QColor("#161618");
        t.panelTitleActive = QColor("#1e1e1e");
        t.raised = QColor("#2c2c2e");
        t.group = QColor("#262628");
        t.groupBorder = QColor("#3a3a3c");
        t.well = QColor("#1e1e1e");
        t.dialog = QColor("#1e1e1e");

        t.dockSeparator = QColor("#3a3a3c");
        t.dockSeparatorHover = QColor("#5e5e62");
        t.divider = QColor("#3a3a3c");
        t.borderSubtle = QColor("#333335");
        t.border = QColor("#48484a");
        t.borderStrong = QColor("#5e5e62");

        t.text = QColor("#dcdcdf");
        t.textStrong = QColor("#f5f5f7");
        t.textSecondary = QColor("#a5a5a8");
        t.textTertiary = QColor("#7c7c80");
        t.placeholderText = QColor("#98989d");
        t.textDisabled = QColor("#6e6e73");
        t.tabInactiveText = QColor("#a5a5a8");

        t.selectionInactive = QColor("#464649");
        t.selectionInactiveText = QColor("#f5f5f7");
        t.hover = rgba(255, 255, 255, 20);
        t.pressed = rgba(255, 255, 255, 34);

        t.field = QColor("#2c2c2e");
        t.fieldBorder = QColor("#48484a");
        t.fieldDisabled = QColor("#252527");
        t.checkFill = QColor("#3a3a3c");
        t.checkBorder = QColor("#7c7c80");

        // The launcher's bezels.
        t.button = QColor("#58585b");
        t.buttonEdge = QColor("#58585b");
        t.buttonTop = QColor("#6c6c70");
        t.buttonBottom = QColor("#4e4e51");
        t.buttonHover = QColor("#626266");
        t.buttonPressed = QColor("#6c6c70");
        t.buttonText = QColor("#f5f5f7");
        t.buttonDisabled = QColor("#3b3b3d");
        t.buttonDisabledEdge = QColor("#444447");
        t.buttonDisabledText = QColor("#8a8a8e");

        t.menu = QColor("#2c2c2e");
        t.menuBorder = QColor("#48484a");
        t.menuText = QColor("#f5f5f7");
        t.menuSeparator = QColor("#3a3a3c");

        t.tooltip = QColor("#2c2c2e");
        t.tooltipBorder = QColor("#48484a");
        t.tooltipText = QColor("#f5f5f7");

        t.scrollHandle = rgba(255, 255, 255, 77);
        t.scrollHandleHover = rgba(255, 255, 255, 115);

        t.tabText = QColor("#a5a5a8");
        t.tabSelected = QColor("#3a3a3c");
        t.tabSelectedBorder = QColor("#48484a");
        t.tabSelectedText = QColor("#f5f5f7");

        t.sliderGroove = QColor("#3a3a3c");
        t.sliderGrooveBorder = QColor("#48484a");
        t.sliderHandle = QColor("#e8e8ed");
        t.sliderHandleBorder = QColor("#8e8e93");

        t.statusBar = QColor("#2c2c2e");
        t.statusText = QColor("#98989d");
        t.statusDivider = QColor("#3a3a3c");

        t.error = QColor("#ff6961");
        t.errorSurface = QColor("#3a2426");
        t.errorBorder = QColor("#7d4045");
        t.errorText = QColor("#ffb4ae");
        // Changed on disk: a neutral bar a step lighter than the toolbar it
        // hangs from; only its triangle is orange.
        t.warning = QColor("#ff9f5a");
        t.warningSurface = QColor("#353537");
        t.warningBorder = QColor("#3a3a3c");
        t.warningText = rgba(235, 235, 245, 204);
        t.warningTitle = QColor("#f5f5f7");
        t.warningGlyph = QColor("#ff9f0a");
        t.success = QColor("#30d158");

        t.assistant = QColor("#bf5af2");
        t.assistantSurface = QColor("#34243d");
        t.assistantBorder = QColor("#64377b");
        t.assistantText = QColor("#dfacf8");

        t.messageUser = QColor("#3a3a3c");
        t.messageUserBorder = QColor("#48484a");
        t.messageAgent = QColor("#2c2c2e");
        t.messageAgentBorder = QColor("#3a3a3c");
        t.messageText = QColor("#e5e5ea");
        t.messageUserRole = QColor("#bfbfc4");

        t.glyph = QColor("#dcdcdf");
        t.glyphDisabled = QColor("#6e6e73");
        t.toolSelected = QColor("#3a3a3c");

        t.timelineBackground = QColor("#1c1c1e");
        t.ruler = QColor("#2c2c2e");
        t.rulerBottom = QColor("#29292b");
        t.rulerTick = QColor("#6e6e73");
        t.rulerTickHalf = QColor("#5e5e62");
        t.rulerTickMinor = QColor("#48484a");
        t.rulerText = QColor("#b0b0b5");
        t.rulerCorner = QColor("#232325");
        t.trackHeader = QColor("#2a2a2c");
        t.trackHeaderEdge = QColor("#333335");
        t.trackHeaderColumn = QColor("#232325");
        t.trackHeaderText = QColor("#e5e5ea");
        t.trackHeaderTextDisabled = QColor("#7c7c80");
        t.trackHeaderDetail = QColor("#9a9a9a");
        t.trackResizeGrip = QColor("#8e8e93");
        t.trackBadge = QColor("#3a3a3c");
        t.trackBadgeText = QColor("#ececec");
        t.trackToggle = QColor("#3a3a3c");
        t.trackToggleGlyph = QColor("#aeaeb2");
        t.trackLane = QColor("#232325");
        t.trackLaneAlt = QColor("#262628");
        t.trackLaneLocked = rgba(255, 255, 255, 14);
        t.trackDivider = QColor("#161618");
        t.trackBankDivider = QColor("#0e0e10");
        t.headerPreview = QColor("#2c2c2e");
        t.headerPreviewBorder = QColor("#161618");
        t.addLane = QColor("#232325");
        t.addLaneButton = QColor("#3a3a3c");
        t.addLaneButtonBorder = QColor("#48484a");
        t.addLaneText = QColor("#f5f5f7");

        t.clipVideo = clipStyle("#1b2a3c", "#325985", "#0b1622", "#f5f5f7", "#95bae4");
        t.clipAudio = clipStyle("#17362f", "#246b5d", "#0a1f1b", "#f5f5f7", "#79d8c5");
        t.clipMusic = clipStyle("#1a321d", "#2f6a37", "#0c1d0e", "#f5f5f7", "#81cf8c");
        t.clipGenerator = clipStyle("#391d2c", "#863761", "#210d18", "#f5f5f7", "#e9a5c9");
        // Captions are a neutral role, not gold.
        t.clipCaption = clipStyle("#2e2e30", "#636366", "#1c1c1e", "#f5f5f7", "#d1d1d6");
        t.clipSelectedRing = QColor("#ffffff");
        t.clipSelectedRingOuter = QColor("#000000");
        t.clipTrim = QColor("#ffffff");
        t.clipHighlight = rgba(255, 255, 255, 40);
        t.clipFxBadge = rgba(0, 0, 0, 110);
        t.clipFxBadgeText = QColor("#ededed");
        t.filmstripSeam = rgba(0, 0, 0, 120);
        t.waveCenter = rgba(0, 0, 0, 70);
        t.playheadHalo = rgba(0, 0, 0, 110);
        t.playheadIdle = QColor("#6e6e73");
        t.skimmer = rgba(240, 240, 240, 215);
        t.skimmerText = QColor("#f5f5f7");
        t.scrubTrack = QColor("#3a3a3c");
        t.snapLine = rgba(255, 255, 255, 220);
        t.readout = rgba(28, 28, 30, 240);
        t.readoutBorder = QColor("#48484a");
        t.readoutText = QColor("#f5f5f7");

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
        t.proposalUnderlay = rgba(0, 0, 0, 140);

        t.thumbnail = QColor("#161618");
        t.tileEmpty = QColor("#2a2a2e");
        t.tileEmptyBorder = QColor("#3a3a3e");
        t.tileEmptyText = QColor("#a1a1a6");
        t.cardPlaceholder = QColor("#2a2a2e");
        t.cardPlaceholderText = QColor("#a1a1a6");
        t.badgeNeutral = rgba(0, 0, 0, 150);
        t.badgeNeutralText = QColor("#ffffff");
    } else {
        t.window = QColor("#ececec");
        t.chrome = QColor("#f5f5f5");
        t.panel = QColor("#f6f6f6");
        t.panelTitle = QColor("#e8e8ea");
        t.panelTitleActive = QColor("#f6f6f6");
        t.raised = QColor("#ffffff");
        t.group = rgba(0, 0, 0, 8);
        t.groupBorder = rgba(0, 0, 0, 20);
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
        // #636366 keeps 4.5:1 on the grey window and group fills, not only on white.
        t.textSecondary = QColor("#636366");
        t.textTertiary = QColor("#8e8e93");
        t.placeholderText = QColor("#6e6e73");
        t.textDisabled = QColor("#a5a5aa");
        t.tabInactiveText = QColor("#5f5f64");

        t.selectionInactive = QColor("#dcdcdf");
        t.selectionInactiveText = QColor("#1d1d1f");
        t.hover = rgba(0, 0, 0, 13);
        t.pressed = rgba(0, 0, 0, 26);

        t.field = QColor("#ffffff");
        t.fieldBorder = QColor("#c9c9ce");
        t.fieldDisabled = QColor("#f5f5f7");
        t.checkFill = QColor("#ffffff");
        // 3:1 on the grey dialog and its grouped boxes, not only on white.
        t.checkBorder = QColor("#808085");

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

        t.scrollHandle = rgba(0, 0, 0, 77);
        t.scrollHandleHover = rgba(0, 0, 0, 115);

        t.tabText = QColor("#6e6e73");
        t.tabSelected = QColor("#ffffff");
        t.tabSelectedBorder = QColor("#d1d1d6");
        t.tabSelectedText = QColor("#1d1d1f");

        t.sliderGroove = QColor("#e3e3e6");
        t.sliderGrooveBorder = QColor("#d1d1d6");
        t.sliderHandle = QColor("#ffffff");
        t.sliderHandleBorder = QColor("#b4b4b9");

        t.statusBar = QColor("#f5f5f5");
        // The window's smallest text, so it keeps a margin over 4.5:1.
        t.statusText = QColor("#636366");
        t.statusDivider = QColor("#e0e0e3");

        t.error = QColor("#d70015");
        t.errorSurface = QColor("#fdecee");
        t.errorBorder = QColor("#f3b7bd");
        t.errorText = QColor("#a1000f");
        t.warning = QColor("#c93400");
        t.warningSurface = QColor("#fff6e5");
        t.warningBorder = QColor("#f2d9ab");
        t.warningText = QColor("#1d1d1f");
        t.warningTitle = QColor("#1d1d1f");
        // macOS's high-contrast orange, which reads (4.5:1) on the cream bar.
        t.warningGlyph = QColor("#c93400");
        t.success = QColor("#1e7b34");

        t.assistant = QColor("#af52de");
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
        t.toolSelected = QColor("#e5e5ea");

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
        t.trackHeaderDetail = QColor("#6e6e73");
        t.trackResizeGrip = QColor("#8e8e93");
        t.trackBadge = QColor("#e3e3e6");
        t.trackBadgeText = QColor("#1d1d1f");
        t.trackToggle = QColor("#e3e3e6");
        t.trackToggleGlyph = QColor("#636366");
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
        // A body that stands off the #f4f4f6 lane, as dark's does.
        t.clipCaption = clipStyle("#dedee3", "#aeaeb2", "#8e8e93", "#1d1d1f", "#636366");
        t.clipSelectedRing = QColor("#ffffff");
        t.clipSelectedRingOuter = QColor("#1d1d1f");
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
        // Every marker reads at 3:1 on the light ruler.
        t.markerOrange = QColor("#dc6510");
        t.markerYellow = QColor("#9c8400");
        t.markerGreen = QColor("#2ea043");
        t.markerCyan = QColor("#1593a7");
        t.markerBlue = QColor("#2f76d9");
        t.markerPurple = QColor("#9b51e0");
        t.markerPink = QColor("#e0408a");
        t.markerWhite = QColor("#7c7c80");

        t.proposalRemove = QColor("#d70015");
        t.proposalRemoveHatch = rgba(215, 0, 21, 110);
        t.proposalUnderlay = rgba(255, 255, 255, 200);

        t.thumbnail = QColor("#e3e3e6");
        t.tileEmpty = QColor("#e3e3e6");
        t.tileEmptyBorder = QColor("#e3e3e6");
        t.tileEmptyText = QColor("#5c5c61");
        t.cardPlaceholder = QColor("#ececee");
        t.cardPlaceholderText = QColor("#5c5c61");
        t.badgeNeutral = rgba(0, 0, 0, 150);
        t.badgeNeutralText = QColor("#ffffff");
    }

    // Derived from the accent and the surfaces above. The accent as text
    // reads on each surface it is drawn on: the transports' timecodes on
    // the panel, the timeline's corner time, and the status bar's sync chip.
    // They are all dark or all light, so reading on one never costs another.
    t.accentText = accent;
    for (const QColor &surface : {t.panel, t.rulerCorner, t.statusBar}) t.accentText = readableOn(t.accentText, surface, 4.5);
    t.accentTint = mixColor(t.panel, accent, dark ? 0.26 : 0.16);
    t.textSelection = mixColor(t.field, accent, dark ? 0.45 : 0.28);
    // The marked range is information: 3:1 on the panel around the bar.
    t.inOutRange = readableOn(mixColor(t.scrubTrack, accent, 0.75), t.panel, 3.0);
    // Media badges are solid with white text in both appearances: a deepened
    // accent for Not downloaded, red for Offline, at 4.5:1.
    t.badgeInfo = readableOn(accent, Qt::white, 4.5);
    t.badgeInfoText = QColor("#ffffff");
    t.badgeError = QColor("#c41e2d");
    t.badgeErrorText = QColor("#ffffff");
    t.assistantHover = mixColor(t.assistantSurface, t.assistant, 0.12);
    t.proposalAdd = t.assistant;
    t.proposalAddFill = t.assistant;
    t.proposalAddFill.setAlpha(dark ? 46 : 38);
    t.proposalChange = dark ? t.assistant.lighter(115) : t.assistant.darker(115);
    t.proposalChangeFill = t.proposalChange;
    t.proposalChangeFill.setAlpha(dark ? 36 : 30);
    t.audioTile = t.clipAudio.fill;
    t.audioTileGlyph = t.clipAudio.wave;
    t.dragShadow = rgba(0, 0, 0, 89);

    // The viewer's field is the same in both appearances.
    t.viewerSurround = QColor("#1c1c1c");
    t.viewerCanvas = QColor("#050505");
    t.viewerText = QColor("#a0a0a0");
    t.viewerTextDim = QColor("#8a8a8a");
    t.viewerGlyph = QColor("#b1b1b1");
    t.viewerGlyphRing = QColor("#626262");
    t.viewerGlyphFill = QColor("#181818");
    t.viewerCaption = rgba(15, 15, 15, 225);
    t.viewerCaptionBorder = QColor("#676767");
    t.viewerCaptionText = QColor("#eeeeee");
    // Masks: white over a dark rule reads on any picture, blue ones included.
    t.maskOutline = QColor("#ffffff");
    t.maskOutlineIdle = rgba(255, 255, 255, 179);
    t.maskShadow = rgba(0, 0, 0, 140);
    t.maskFill = rgba(255, 255, 255, 30);
    t.maskHandleFill = accent;
    t.maskHandleBorder = QColor("#ffffff");
    t.maskTag = rgba(0, 0, 0, 153);
    t.maskTagSelected = t.selection;
    t.maskText = QColor("#ffffff");
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
