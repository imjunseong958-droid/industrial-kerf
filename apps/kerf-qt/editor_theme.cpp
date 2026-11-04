#include "editor_theme.h"
#include "design_tokens.h"
#include "editor_style.h"

#include <QApplication>
#include <QEvent>
#include <QGuiApplication>
#include <QHash>
#include <QJsonArray>
#include <QPointer>
#include <QScreen>
#include <QStyleHints>
#include <QWidget>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

namespace tk = tokens;

// A design token as a QColor.
QColor token(QRgb rgba) { return tk::color(rgba); }
// A token at `fraction` opacity (0 to 1), as the design writes rgba().
QColor alphaOf(QRgb rgb, double fraction) {
    QColor color = tk::color(rgb);
    color.setAlpha(tk::alpha(fraction));
    return color;
}
// The translucent `top` as it shows on the opaque `under`: what a seam, a
// wash or a shade drawn on that surface looks like, for roles that are
// filled solid.
QColor laid(QRgb under, QColor top) {
    const qreal amount = top.alphaF();
    top.setAlpha(255);
    return QColor::fromRgba(mixColor(token(under), top, amount).rgba());
}
QColor laid(QRgb under, QRgb top) { return laid(under, token(top)); }
// The same in 1% steps, for a colour that should move no further than it must.
QColor justReadableOn(const QColor &color, const QColor &background, qreal ratio) {
    return readableOn(color, background, ratio, 100);
}

ClipStyle clipStyle(const QColor &fill, const QColor &header, const QColor &border, const QColor &label, const QColor &wave) {
    return {fill, header, border, label, wave};
}

struct AppearanceState {
    EditorTheme theme;
    bool computed = false;
    bool applied = false;
    bool applying = false;
};

AppearanceState &state() {
    static AppearanceState s;
    return s;
}

// Applies the theme again when the application font or a screen's density
// changes, so widgets that cache pixmaps (EditorAppearance::changed) redraw
// them at the new size. The theme itself never changes.
class MetricsWatcher final : public QObject {
public:
    using QObject::QObject;

    void reapply() {
        state().applied = false;
        applyEditorAppearance();
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched == qApp && event->type() == QEvent::ApplicationFontChange) reapply();
        return QObject::eventFilter(watched, event);
    }
};

void watchMetrics() {
    static QPointer<MetricsWatcher> watcher;
    if (watcher || !qApp) return;
    watcher = new MetricsWatcher(qApp);
    qApp->installEventFilter(watcher);
    const auto watchScreen = [](QScreen *screen) {
        QObject::connect(screen, &QScreen::logicalDotsPerInchChanged, watcher, [] { watcher->reapply(); });
    };
    for (QScreen *screen : QGuiApplication::screens()) watchScreen(screen);
    QObject::connect(qApp, &QGuiApplication::screenAdded, watcher, watchScreen);
}

} // namespace

QColor readableOn(const QColor &color, const QColor &background, qreal ratio, int steps) {
    const QColor target = background.lightness() < 128 ? QColor(Qt::white) : QColor(Qt::black);
    QColor result = color;
    // Each step is rounded to 8 bits before it is measured, so the colour
    // returned is one a pixel can hold and still reads at `ratio`.
    for (int step = 1; step <= steps && contrastRatio(result, background) < ratio; ++step)
        result = QColor::fromRgba(mixColor(color, target, qreal(step) / steps).rgba());
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

EditorTheme makeEditorTheme() {
    EditorTheme t;

    t.card = token(tk::card);
    t.on = token(tk::on);
    t.hover2 = token(tk::hover2);
    t.ink = token(tk::ink);
    t.ink2 = token(tk::ink2);
    t.ink3 = token(tk::ink3);
    t.track = token(tk::track);
    t.track2 = token(tk::track2);
    t.line = token(tk::line);
    t.line2 = token(tk::line2);
    t.lineh = token(tk::lineh);
    t.accentHover = token(tk::accentHover);
    t.accentInk = token(tk::accentInk);
    t.pos = token(tk::pos);

    // Accent: the one blue. Text on it is white; as text and thin marks on
    // the ramp it is the light accent ink.
    t.accent = token(tk::accent);
    t.accentPressed = token(tk::accentHover);
    t.accentText = token(tk::accentInk);
    // A chosen tool or mode is the `on` fill (mockup .ib.on), darkening to
    // `raised` while held.
    t.accentTint = token(tk::on);
    t.accentTintPressed = token(tk::raised);
    t.onAccent = token(tk::accentText);
    t.onAccentSecondary = alphaOf(tk::accentText, 0.78);

    // Surfaces, from the back of the window forwards.
    t.window = token(tk::surroundDim);
    t.chrome = token(tk::chrome);
    t.panel = token(tk::chrome);
    t.panelTitle = token(tk::chrome);
    t.panelTitleActive = token(tk::chrome);
    t.raised = token(tk::raised);
    t.group = token(tk::card);
    t.groupBorder = token(tk::line);
    t.well = token(tk::well);
    t.dialog = token(tk::card);

    // Lines: the seams are a 1 pt line drawn inside the panel, so the solid
    // roles are that line as it shows on chrome; control outlines stay
    // translucent so they read on any surface.
    t.divider = laid(tk::chrome, tk::line);
    t.borderSubtle = token(tk::line);
    t.border = token(tk::line2);
    t.borderStrong = token(tk::lineh);

    // Text.
    t.text = token(tk::ink);
    t.textStrong = token(tk::ink);
    t.textSecondary = token(tk::ink2);
    t.placeholderText = token(tk::ink3);
    t.textDisabled = token(tk::dis);
    t.tabInactiveText = token(tk::ink2);

    // Selection: a focused list's chosen row is the accent; without focus
    // it is the `on` fill.
    t.selection = token(tk::accent);
    t.selectionText = token(tk::accentText);
    t.selectionInactive = token(tk::on);
    t.selectionInactiveText = token(tk::ink);
    t.textSelection = laid(tk::raised, alphaOf(tk::accent, 0.45));
    t.hover = alphaOf(0xffffffff, 0.06);

    // Fields: inputs on `raised` with a line2 inset, the accent in focus.
    t.field = token(tk::raised);
    t.fieldBorder = token(tk::line2);
    t.fieldDisabled = token(tk::card);
    t.focusRing = t.accent;
    t.focusHalo = t.accent; // The 2 pt outline, solid (mockup: outline 2px accent).
    // Check boxes: no fill, a 1 pt outline at 16% white; checked, `on`.
    t.checkBorder = laid(tk::chrome, tk::checkboxOutline);

    // Buttons: the secondary button is the raised fill with a line2 edge,
    // flat (top and bottom edges the same); hover2 under the pointer, `on`
    // while held.
    t.button = token(tk::raised);
    t.buttonEdge = token(tk::line2);
    t.buttonTop = token(tk::line2);
    t.buttonBottom = token(tk::line2);
    t.buttonHover = token(tk::hover2);
    t.buttonPressed = token(tk::on);
    t.buttonText = token(tk::ink);
    t.buttonDisabled = token(tk::chrome);
    t.buttonDisabledEdge = token(tk::line);
    t.buttonDisabledText = token(tk::dis);

    // Menus and tooltips (0.5): cards with a line2 inset; the row under the
    // pointer is `raised` with the same ink.
    t.menu = token(tk::card);
    t.menuBorder = token(tk::line2);
    t.menuText = token(tk::ink);
    t.menuSelection = token(tk::raised);
    t.menuSelectionText = token(tk::ink);
    t.menuSeparator = laid(tk::card, tk::line);
    t.tooltip = token(tk::card);
    t.tooltipBorder = token(tk::line2);
    t.tooltipText = token(tk::ink);
    // Scroll bars are hidden (the style sheet gives them no width); a
    // widget that shows one of its own uses these.
    t.scrollHandle = alphaOf(0xffffffff, 0.40);
    t.scrollHandleHover = alphaOf(0xffffffff, 0.70);

    t.tabText = token(tk::ink2);
    t.tabSelected = token(tk::on);
    t.tabSelectedBorder = token(tk::line2);
    t.tabSelectedText = token(tk::ink);

    // Sliders (.sl): a 2 pt track, a track2 fill and an ink knob.
    t.sliderGroove = token(tk::track);
    t.sliderHandle = token(tk::ink);

    // Status: neg, warn and pos; a banner is its colour washed into chrome.
    t.error = token(tk::neg);
    t.errorSurface = laid(tk::chrome, alphaOf(tk::neg, 0.12));
    t.errorBorder = laid(tk::chrome, alphaOf(tk::neg, 0.35));
    t.errorText = token(tk::neg);
    t.warning = token(tk::warn);
    t.warningSurface = laid(tk::chrome, alphaOf(tk::warn, 0.10));
    t.warningBorder = laid(tk::chrome, alphaOf(tk::warn, 0.30));
    t.warningText = token(tk::ink2);
    t.warningTitle = token(tk::ink);
    t.warningGlyph = token(tk::warn);
    t.success = token(tk::pos);

    // Kerf's work: proposals and Kerf's marks are the accent family; the
    // proposal card is `raised`.
    t.assistant = t.accent;
    t.assistantSurface = token(tk::raised);
    t.assistantText = token(tk::accentInk);
    t.assistantHover = token(tk::hover2);

    // The comments panel (4): a card; chips and pills on `raised`.
    t.chatSurface = token(tk::chrome);
    t.chatText = token(tk::ink);
    t.chatSecondary = token(tk::ink2);
    t.chatChip = token(tk::raised);
    t.chatChipHover = token(tk::hover2);
    t.chatChipText = token(tk::ink);
    t.chatAvatarUser = token(tk::on);
    t.chatAvatarUserText = token(tk::ink2);
    t.chatAvatarKerf = t.accent;
    t.chatAvatarKerfText = token(tk::accentText);
    t.chatExpander = token(tk::accentInk);
    t.chatComposerRuleFocus = t.accent;
    t.chatScrollHandle = alphaOf(0xffffffff, 0.40);
    t.chatScrollHandleHover = alphaOf(0xffffffff, 0.70);
    t.messageUser = t.chatSurface;
    t.messageAgent = t.chatSurface;
    t.messageUserRole = t.chatText;
    t.pillTonal = token(tk::raised);
    t.pillTonalText = token(tk::ink);

    t.glyph = token(tk::ink2);
    t.glyphDisabled = token(tk::dis);
    // The chosen tool in the tool palette: `raised`, where the accent glyph
    // the palette draws on it still reads at 3:1 (on `on` it would not).
    // W2's segmented tools use the on fill with an ink glyph instead.
    t.toolSelected = token(tk::raised);

    // Timeline (7): the ruler's chrome, which the playhead and markers
    // must read on (the guards below). The timeline paints the rest from
    // design_tokens.h.
    t.ruler = token(tk::chrome);
    t.rulerBottom = token(tk::chrome);
    t.rulerCorner = token(tk::chrome);

    // Clips (7.4): a card with a line2 inset and an ink name; a picture's
    // name bar is black at 55% over its filmstrip; titles and captions are
    // `raised`. Sound draws its waveform in the literal greys.
    const QColor cardLine = laid(tk::card, tk::line2);
    const QColor raisedLine = laid(tk::raised, tk::line2);
    const QColor wave = token(tk::overlay::waveformRms);
    t.clipVideo = clipStyle(token(tk::card), laid(tk::card, alphaOf(0xff000000, 0.55)), cardLine, token(tk::ink), wave);
    t.clipAudio = clipStyle(token(tk::card), token(tk::card), cardLine, token(tk::ink), wave);
    t.clipMusic = clipStyle(token(tk::card), token(tk::card), cardLine, token(tk::ink), wave);
    t.clipGenerator = clipStyle(token(tk::raised), token(tk::raised), raisedLine, token(tk::ink), token(tk::ink2));
    t.clipCaption = clipStyle(token(tk::raised), token(tk::raised), raisedLine, token(tk::ink2), token(tk::ink2));
    // The playhead is its own red, stroked in the stage's dark.
    t.playhead = token(tk::playhead);
    t.playheadHalo = token(tk::surround);
    // The skimmed playhead: track2, which reads at 3:1 on the ruler and its
    // halo but clearly under the playhead.
    t.playheadIdle = token(tk::track2);
    t.inOutRange = t.accent; // The selected range.

    // Marker colours are the document's names; each is lifted to read on
    // the ruler (the guard below).
    t.markerRed = token(tk::neg);
    t.markerOrange = cssColor("#e0904a");
    t.markerYellow = token(tk::warn);
    t.markerGreen = token(tk::pos);
    t.markerCyan = cssColor("#5cb8c4");
    t.markerBlue = token(tk::accentInk);
    t.markerPurple = cssColor("#b48ce6");
    t.markerPink = cssColor("#e07ab0");
    t.markerWhite = token(tk::ink);

    // A pending proposal's changed clip: the accent ink.
    t.proposalChange = token(tk::accentInk);

    // Viewers (3): the surround, a black canvas, the literal caption.
    t.viewerSurround = token(tk::surround);
    t.viewerCanvas = token(tk::overlay::deviceScreen);
    t.viewerText = token(tk::ink2);
    t.viewerTextDim = token(tk::ink3);
    t.viewerCaption = token(tk::overlay::captionBackground);
    t.viewerCaptionBorder = t.viewerCaption; // No box line around the caption.
    t.deviceScreen = token(tk::overlay::deviceScreen);
    // Masks over the picture: white over a dark rule; handles the accent.
    t.maskOutline = token(0xffffffff);
    t.maskOutlineIdle = alphaOf(0xffffffff, 0.70);
    t.maskShadow = alphaOf(0xff000000, 0.55);
    t.maskFill = alphaOf(0xffffffff, 0.12);
    t.maskHandleFill = t.accent;
    t.maskHandleBorder = token(0xffffffff);
    t.maskTag = alphaOf(0xff000000, 0.60);
    t.maskTagSelected = t.accent;
    t.maskText = token(0xffffffff);

    // The bin and the browser (6, 8): tiles on the well.
    t.tileEmpty = token(tk::card);
    t.tileEmptyText = token(tk::ink2);
    t.cardPlaceholder = token(tk::raised);
    t.cardPlaceholderText = token(tk::ink2);
    t.badgeError = token(tk::neg);
    t.badgeErrorText = token(tk::surroundDim);
    t.badgeInfo = t.accent;
    t.badgeInfoText = token(tk::accentText);

    // The Inspector's controls: aliases.
    t.inspectorField = t.field;
    t.switchOff = token(tk::track);
    t.switchOffStroke = token(tk::line2);
    t.knob = token(tk::ink);

    // Guards: each holds for the values above, and keeps holding if a token
    // is retuned. Text reads at 4.5:1 where it is drawn...
    t.onAccent = readableOn(t.onAccent, t.accent, 4.5);
    t.onAccentSecondary = readableAlphaOn(t.onAccentSecondary, t.selection, 4.5);
    t.selectionText = readableOn(t.selectionText, t.selection, 4.5);
    t.menuSelectionText = readableOn(t.menuSelectionText, t.menuSelection, 4.5);
    for (const QColor &surface : {t.field, t.chatSurface, t.dialog})
        t.placeholderText = justReadableOn(t.placeholderText, surface, 4.5);
    t.viewerTextDim = justReadableOn(t.viewerTextDim, t.viewerCanvas, 4.5);
    t.errorText = justReadableOn(t.errorText, t.errorSurface, 4.5);
    t.errorText = justReadableOn(t.errorText, t.chatSurface, 4.5);
    t.badgeErrorText = readableOn(t.badgeErrorText, t.badgeError, 4.5);
    // ...the accent as text on every surface it is drawn on...
    for (const QColor &surface : {t.panel, t.rulerCorner, t.raised, t.window, t.chrome, t.well, t.field,
                                  t.dialog, t.group, t.chatSurface, t.accentTint, t.accentTintPressed})
        t.accentText = readableOn(t.accentText, surface, 4.5);
    t.chatExpander = t.accentText;
    t.assistantText = t.accentText;
    // ...the marked range is information, 3:1 on the panel round the bar...
    t.inOutRange = readableOn(t.inOutRange, t.panel, 3.0);
    // ...the skimmed playhead reads at 3:1 on the ruler and on its halo, no
    // more than it must; the playhead clearly more...
    for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
        t.playheadIdle = justReadableOn(t.playheadIdle, surface, 3.0);
    for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
        t.playhead = justReadableOn(t.playhead, surface, std::max(3.0, contrastRatio(t.playheadIdle, surface) + 0.4));
    // ...every marker reads at 3:1 on the whole ruler...
    for (QColor *marker : {&t.markerRed, &t.markerOrange, &t.markerYellow, &t.markerGreen, &t.markerCyan, &t.markerBlue,
                           &t.markerPurple, &t.markerPink, &t.markerWhite})
        for (const QColor &surface : {t.ruler, t.rulerBottom}) *marker = readableOn(*marker, surface, 3.0);
    // ...every clip's name reads on its header at 4.5:1...
    for (ClipStyle *style : {&t.clipVideo, &t.clipAudio, &t.clipMusic, &t.clipGenerator, &t.clipCaption})
        style->label = readableOn(style->label, style->header, 4.5);
    return t;
}

QPalette editorPalette(const EditorTheme &t) {
    QPalette palette;
    for (const QPalette::ColorGroup group : {QPalette::Active, QPalette::Inactive}) {
        palette.setColor(group, QPalette::Window, t.chrome);
        palette.setColor(group, QPalette::WindowText, t.text);
        palette.setColor(group, QPalette::Base, t.field);
        palette.setColor(group, QPalette::AlternateBase, t.well);
        palette.setColor(group, QPalette::Text, t.text);
        palette.setColor(group, QPalette::Button, t.button);
        palette.setColor(group, QPalette::ButtonText, t.buttonText);
        palette.setColor(group, QPalette::BrightText, t.textStrong);
        palette.setColor(group, QPalette::ToolTipBase, t.tooltip);
        palette.setColor(group, QPalette::ToolTipText, t.tooltipText);
        palette.setColor(group, QPalette::PlaceholderText, t.placeholderText);
        palette.setColor(group, QPalette::Link, t.accentText);
        palette.setColor(group, QPalette::Light, t.buttonHover);
        palette.setColor(group, QPalette::Midlight, t.raised);
        palette.setColor(group, QPalette::Mid, t.divider);
        palette.setColor(group, QPalette::Dark, t.well);
        palette.setColor(group, QPalette::Shadow, QColor(Qt::black));
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
        palette.setColor(group, QPalette::Accent, t.accent);
#endif
    }
    palette.setColor(QPalette::Active, QPalette::Highlight, t.selection);
    palette.setColor(QPalette::Active, QPalette::HighlightedText, t.selectionText);
    palette.setColor(QPalette::Inactive, QPalette::Highlight, t.selectionInactive);
    palette.setColor(QPalette::Inactive, QPalette::HighlightedText, t.selectionInactiveText);
    for (const QPalette::ColorRole role : {QPalette::WindowText, QPalette::Text, QPalette::ButtonText})
        palette.setColor(QPalette::Disabled, role, t.textDisabled);
    palette.setColor(QPalette::Disabled, QPalette::Window, t.chrome);
    palette.setColor(QPalette::Disabled, QPalette::Base, t.fieldDisabled);
    palette.setColor(QPalette::Disabled, QPalette::Button, t.buttonDisabled);
    palette.setColor(QPalette::Disabled, QPalette::Highlight, t.selectionInactive);
    palette.setColor(QPalette::Disabled, QPalette::HighlightedText, t.textDisabled);
    return palette;
}

const EditorTheme &editorTheme() {
    AppearanceState &s = state();
    if (!s.computed) {
        s.theme = makeEditorTheme();
        s.computed = true;
    }
    return s.theme;
}

void applyEditorAppearance() {
    AppearanceState &s = state();
    if (s.applying || !qApp || s.applied) return;
    s.applying = true;
    editorTheme();
    s.applied = true;
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    // Native parts (title bars, file and colour dialogs) draw dark too.
    QGuiApplication::styleHints()->setColorScheme(Qt::ColorScheme::Dark);
#endif
    QApplication::setPalette(editorPalette(s.theme));
    // Controls are 12/16 unless a widget says otherwise.
    QApplication::setFont(tokens::font(tokens::control));
    qApp->setStyleSheet(editorStyleSheet(s.theme));
    // The style sheet repolishes every widget; painted ones read the theme
    // as they paint.
    for (QWidget *widget : QApplication::allWidgets()) widget->update();
    watchMetrics();
    emit EditorAppearance::instance()->changed();
    s.applying = false;
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
