// Tests for the editor theme: the dark graphite ramp, type scale, contrast,
// focus outlines and the flat style sheet.

#include "fixtures.h"
#include "ui/components.h"
#include "design_tokens.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
#include "proposal_card.h"
#include "comment_row.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"

#include <QAbstractItemView>
#include <QAccessible>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMenuBar>
#include <QMimeData>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QScreen>
#include <QScrollArea>
#include <QScrollBar>
#include <QSlider>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QVBoxLayout>
#include <QWheelEvent>
#include <QWidget>


#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

class ThemeTest final : public QObject {
    Q_OBJECT

private slots:
    // A colour or size on the app sheet's * rule beats every label's own
    // palette and setFont: role names and times read as bright 12 pt text.
    void aLabelKeepsItsOwnInkAndSize() {
        applyEditorAppearance();
        QLabel label(QStringLiteral("HHHHHHHH"));
        label.setFont(tokens::font(tokens::meta));
        QPalette palette = label.palette();
        palette.setColor(QPalette::WindowText, tokens::color(tokens::ink3));
        label.setPalette(palette);
        label.ensurePolished();
        QCOMPARE(label.font().pixelSize(), tokens::meta.px);
        label.resize(label.sizeHint());
        QImage image(label.size(), QImage::Format_RGB32);
        image.fill(Qt::black);
        label.render(&image);
        int brightest = 0;
        for (int y = 0; y < image.height(); ++y)
            for (int x = 0; x < image.width(); ++x) brightest = std::max(brightest, qGray(image.pixel(x, y)));
        QVERIFY(brightest > 0);
        QVERIFY2(brightest <= qGray(tokens::ink3), qPrintable(QString::number(brightest)));
    }

    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // Whether `combo`, in `shot` of its window, is drawn as the design draws
    // a pop-up button: a secondary button's bezel, filled with `button`, flat
    // (its top and bottom edges the same), not a field's.
    static bool popUpHasButtonBezel(const QImage &shot, const QComboBox *combo, const QColor &button) {
        const qreal ratio = shot.devicePixelRatio();
        const QRect bezel(combo->mapTo(combo->window(), QPoint()), combo->size());
        const auto at = [&](int x, int y) { return shot.pixelColor(QPoint(x, y) * ratio); };
        return at(bezel.left() + 4, bezel.center().y()).rgb() == button.rgb() &&
               at(bezel.center().x(), bezel.bottom()).rgb() == at(bezel.center().x(), bezel.top()).rgb();
    }

    // The most contrast any pixel of `area` in `image` has with `background`.
    static qreal highestContrast(const QImage &image, const QRect &area, const QColor &background) {
        qreal best = 1.0;
        const QRect inside = area.intersected(image.rect());
        for (int y = inside.top(); y <= inside.bottom(); ++y)
            for (int x = inside.left(); x <= inside.right(); ++x)
                best = std::max(best, contrastRatio(image.pixelColor(x, y), background));
        return best;
    }

    // Kerf has its own accent, the design's one blue, and does not follow
    // the system's: the editor and the launcher keep it when the platform
    // reports another accent.
    void kerfDrawsItsOwnBlueAccent() {
        const auto restore = qScopeGuard([] { QApplication::setPalette(editorPalette(editorTheme())); });
        const EditorTheme t = makeEditorTheme();
        const LauncherTheme l = launcherTheme();
        QCOMPARE(t.accent.rgb(), QColor("#3366e6").rgb());
        QCOMPARE(t.accent.rgb(), tokens::color(tokens::accent).rgb());
        QCOMPARE(l.accent.rgb(), t.accent.rgb());
        // Kerf's own marks are the accent: focus, the agent's work, its
        // avatar and the mask handles. The playhead is the design's red.
        for (const QColor &mark : {t.focusRing, t.assistant, t.chatAvatarKerf, t.maskHandleFill})
            QCOMPARE(mark.rgb(), t.accent.rgb());
        QCOMPARE(t.playhead.rgb(), QColor("#e5484d").rgb());
        // The accent as text is the accent ink.
        QCOMPARE(t.accentText.rgb(), QColor("#8fb0ff").rgb());
        QVERIFY(contrastRatio(l.selectionText, l.selection) >= 4.5);
        // Fewer boxes: the caption on the picture has no line round it; a
        // rail button is clear at rest and the on fill when on (.ib.on), its
        // symbol the ink; the launcher's format tiles are raised on the page.
        QCOMPARE(t.viewerCaptionBorder.rgba(), t.viewerCaption.rgba());
        QVERIFY(l.tile.lightness() > l.content.lightness());
        // Clips are graphite: no clip kind's header has any colour in it,
        // and every label reads on its header.
        for (const ClipStyle *style : {&t.clipVideo, &t.clipAudio, &t.clipMusic, &t.clipGenerator, &t.clipCaption}) {
            QVERIFY2(isGraphite(style->header), qPrintable(style->header.name()));
            QVERIFY(contrastRatio(style->label, style->header) >= 4.5);
        }
        QVERIFY(t.clipVideo.header.rgb() != t.clipCaption.header.rgb());
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
        // A system accent of any colour leaves Kerf's in place.
        QPalette red = QGuiApplication::palette();
        red.setColor(QPalette::Accent, QColor("#ff2d55"));
        QGuiApplication::setPalette(red);
        applyEditorAppearance();
        QCOMPARE(editorTheme().accent.rgb(), QColor("#3366e6").rgb());
        ProjectLauncher launcher;
        QCOMPARE(launcher.theme().accent.rgb(), QColor("#3366e6").rgb());
        // A focused list's chosen row is Kerf's accent.
        QCOMPARE(editorPalette(editorTheme()).color(QPalette::Active, QPalette::Highlight).rgb(), t.accent.rgb());
#endif
    }

    // The accent as text (the transports' timecodes, the timeline's corner
    // time, the sync chip) reads at 4.5:1 on each surface it is drawn on;
    // text on the filled accent reads at 4.5:1 too, with no exemption.
    void accentTextReadsOnEverySurfaceItIsDrawnOn() {
        const EditorTheme theme = makeEditorTheme();
        for (const QColor &surface : {theme.panel, theme.rulerCorner, theme.raised, theme.window,
                                      theme.chrome, theme.well, theme.field, theme.dialog, theme.group, theme.chatSurface,
                                      theme.accentTint, theme.accentTintPressed}) {
            const qreal ratio = contrastRatio(theme.accentText, surface);
            QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1 on %2: %3").arg(theme.accentText.name(), surface.name()).arg(ratio, 0, 'f', 2)));
        }
        // So does the Source monitor's marked range, at 3:1 on its panel.
        QVERIFY(contrastRatio(theme.inOutRange, theme.panel) >= 3.0);
        // Text on the accent's fill, at rest, under the pointer and held.
        for (const QColor &fill : {theme.accent, theme.accentPressed, theme.accentHover})
            QVERIFY2(contrastRatio(theme.onAccent, fill) >= 4.5, qPrintable(fill.name()));
    }

    // The chrome keeps the design's one type scale (mockup-spec 0.3): every
    // size the editor's, the docks' and the launcher's style sheets set is
    // 11, 12, 13 or 14px, or 15px (the caption on the picture only).
    void chromeKeepsOneTypeScale() {
        const QSet<int> scale{11, 12, 13, 14, 15};
        static const QRegularExpression rule(QStringLiteral("([^{}]+)\\{([^{}]*)\\}"));
        static const QRegularExpression size(QStringLiteral("font-size:\\s*(\\d+)px"));
        const EditorTheme t = makeEditorTheme();
        for (const QString &sheet : {editorStyleSheet(t), launcherStyleSheet(launcherTheme())}) {
            for (auto it = rule.globalMatch(sheet); it.hasNext();) {
                const QRegularExpressionMatch match = it.next();
                const QString selector = match.captured(1).trimmed();
                for (auto sizes = size.globalMatch(match.captured(2)); sizes.hasNext();) {
                    const int pixels = sizes.next().captured(1).toInt();
                    QVERIFY2(scale.contains(pixels), qPrintable(QStringLiteral("%1px in %2").arg(pixels).arg(selector)));
                    if (pixels == 15) QVERIFY2(selector.contains(QStringLiteral("aption")), qPrintable(selector));
                }
            }
        }
        // The type pairs themselves.
        QCOMPARE(tokens::body.px, 13);
        QCOMPARE(tokens::control.px, 12);
        QCOMPARE(tokens::meta.px, 11);
        QCOMPARE(tokens::title.px, 14);
        QCOMPARE(tokens::caption.px, 15);
        // Mono figures are the tokens' mono face.
        const QFont mono = monoFont(12);
        QCOMPARE(mono.pixelSize(), 12);
        QVERIFY(mono.families().contains(QStringLiteral("JetBrains Mono")));
        QVERIFY(mono.families().contains(QStringLiteral("Menlo")));
    }

    // Colours drawn one on another, each with the ratio it must reach there
    // (4.5:1 for text, 3:1 for symbols, edges and handles); a translucent one
    // is laid on what is under it first.
    void themeColorsReadWhereTheyAreDrawn_data() {
        QTest::addColumn<QColor>("foreground");
        QTest::addColumn<QColor>("background");
        QTest::addColumn<qreal>("ratio");
        struct Pair {
            QByteArray what;
            QColor foreground;
            QColor background;
            qreal ratio;
        };
        {
            const EditorTheme t = makeEditorTheme();
            const LauncherTheme l = launcherTheme();
            std::vector<Pair> pairs{
                {"chosen mask tool's symbol on its wash", t.accentText, t.accentTint, 3.0},
                {"chosen mask tool's symbol on its wash, held", t.accentText, t.accentTintPressed, 3.0},
                {"selected mask's name on its wash", t.buttonText, t.accentTint, 4.5},
                {"selected mask's name on its wash, held", t.buttonText, t.accentTintPressed, 4.5},
                {"playhead's head on the ruler's top", t.playhead, t.ruler, 3.0},
                {"playhead's head on the ruler's foot", t.playhead, t.rulerBottom, 3.0},
                {"playhead's line on its halo", t.playhead, t.playheadHalo, 3.0},
                {"text on a selected row", t.selectionText, t.selection, 4.5},
                {"secondary text on a selected row", t.onAccentSecondary, t.selection, 4.5},
                {"chosen menu item", t.menuSelectionText, t.menuSelection, 4.5},
                {"default button's label", t.onAccent, t.accent, 4.5},
                {"check box's tick", t.ink2, t.on, 3.0},
                {"tertiary text on chrome", t.ink3, t.chrome, 4.5},
                {"primary button's label under the pointer", t.onAccent, t.accentHover, 4.5},
                {"picture clip's name", t.clipVideo.label, t.clipVideo.header, 4.5},
                {"sound clip's name", t.clipAudio.label, t.clipAudio.header, 4.5},
                {"music clip's name", t.clipMusic.label, t.clipMusic.header, 4.5},
                {"title clip's name", t.clipGenerator.label, t.clipGenerator.header, 4.5},
                {"caption clip's name", t.clipCaption.label, t.clipCaption.header, 4.5},
                {"Model field under the pointer", t.text, laidOn(t.field, t.hover), 4.5},
                {"Model field under the pointer while sending", t.text, laidOn(t.assistantSurface, t.hover), 4.5},
                {"disclosure chevron on a dialog", t.textSecondary, t.dialog, 3.0},
                {"disclosure chevron on a dialog's group", t.textSecondary, laidOn(t.dialog, t.raised), 3.0},
                {"disclosure chevron on a panel", t.textSecondary, t.panel, 3.0},
                {"disclosure chevron on a well", t.textSecondary, t.well, 3.0},
                {"scroll bar handle on a panel", t.scrollHandle, t.panel, 3.0},
                {"scroll bar handle on a well", t.scrollHandle, t.well, 3.0},
                {"scroll bar handle on a field", t.scrollHandle, t.field, 3.0},
                {"scroll bar handle on a dialog", t.scrollHandle, t.dialog, 3.0},
                {"skimmed playhead's head on the ruler's top", t.playheadIdle, t.ruler, 3.0},
                {"skimmed playhead's head on the ruler's foot", t.playheadIdle, t.rulerBottom, 3.0},
                {"skimmed playhead's line on its halo", t.playheadIdle, t.playheadHalo, 3.0},
                {"You on the user's message", t.messageUserRole, t.messageUser, 4.5},
                {"unreadable preset on the preset list", t.error, t.well, 4.5},
                {"Source monitor's message on the stage", t.textSecondary, t.viewerSurround, 4.5},
                {"chat's secondary text", t.chatSecondary, t.chatSurface, 4.5},
                {"chat's reply expander", t.chatExpander, t.chatSurface, 4.5},
                {"Kerf's monogram on its avatar", t.chatAvatarKerfText, t.chatAvatarKerf, 4.5},
                {"the user's initials on their avatar", t.chatAvatarUserText, t.chatAvatarUser, 4.5},
                {"chip's label on its wash", t.chatChipText, laidOn(t.chatSurface, t.chatChip), 4.5},
                {"chat's text on its surface", t.chatText, t.chatSurface, 4.5},
                {"chat's error tag", t.errorText, t.chatSurface, 4.5},
                {"chat's header buttons' symbols on their wash", t.chatText, laidOn(t.chatSurface, t.chatChipHover), 3.0},
                {"a comment's action under the pointer", t.chatText, laidOn(t.chatSurface, t.chatChip), 4.5},
                {"a comment's quiet action", t.chatSecondary, t.chatSurface, 4.5},
                {"chip's label under the pointer", t.chatChipText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"chip's label on the timeline's bar", t.chatChipText, laidOn(t.chrome, t.chatChip), 4.5},
                {"Send and Accept's label on the accent", t.onAccent, t.accent, 4.5},
                {"Reject and Stop's label on their tonal fill", t.pillTonalText, laidOn(t.chatSurface, t.pillTonal), 4.5},
                {"Reject and Stop's label under the pointer", t.pillTonalText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"the composer's underline in focus", t.chatComposerRuleFocus, t.chatSurface, 3.0},
                {"the composer's text", t.chatText, t.chatSurface, 4.5},
                {"the composer's placeholder", t.placeholderText, t.chatSurface, 4.5},
                {"the model's name under the pointer", t.chatText, laidOn(t.chatSurface, t.chatChipHover), 4.5},
                {"launcher: Details chevron on the content", l.secondary, l.content, 3.0},
                {"launcher: scroll bar handle on the content", l.scrollHandle, l.content, 3.0},
                {"launcher: scroll bar handle on a group", l.scrollHandle, l.group, 3.0},
                {"launcher: scroll bar handle on the sidebar", l.scrollHandle, l.sidebar, 3.0},
                {"launcher: missing project on an inactive selection", l.secondaryOnSelection, l.selectionInactive, 4.5},
                {"launcher: text on a selected row", l.selectionText, l.selection, 4.5},
                {"launcher: secondary text on a selected row", l.onAccentSecondary, l.selection, 4.5},
                {"launcher: Choose's label", l.onAccent, l.accent, 4.5},
                {"launcher: Choose's label, held", l.onAccent, l.accentPressed, 4.5},
                {"launcher: sidebar symbol", l.accent, l.sidebar, 3.0},
                {"launcher: format tile's frame", l.tileGlyph, l.tile, 3.0},
                {"launcher: format tile's plus", l.tileGlyph, l.badge, 3.0},
                {"launcher: switch's knob on the accent", l.knobOn, l.accent, 3.0},
                {"launcher: tooltip", l.tooltipText, l.tooltip, 4.5},
            };
            for (const QByteArray marker : {"red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white", ""}) {
                const QColor color = t.marker(QString::fromLatin1(marker));
                const QByteArray name = marker.isEmpty() ? QByteArray("uncoloured") : marker;
                pairs.push_back({name + " marker on the ruler's top", color, t.ruler, 3.0});
                pairs.push_back({name + " marker on the ruler's foot", color, t.rulerBottom, 3.0});
            }
            for (const Pair &pair : pairs)
                QTest::addRow("%s (%s)", pair.what.constData(), qPrintable(pair.foreground.name(QColor::HexArgb)))
                    << pair.foreground << pair.background << pair.ratio;
        }
    }

    void themeColorsReadWhereTheyAreDrawn() {
        QFETCH(QColor, foreground);
        QFETCH(QColor, background);
        QFETCH(qreal, ratio);
        const QColor drawn = laidOn(background, foreground);
        const qreal measured = contrastRatio(drawn, background);
        QVERIFY2(measured >= ratio, qPrintable(QStringLiteral("%1 on %2: %3, under %4")
                                                   .arg(drawn.name(), background.name())
                                                   .arg(measured, 0, 'f', 2)
                                                   .arg(ratio, 0, 'f', 2)));
    }

    // States meant to differ still do: the skimmed playhead is a grey that
    // reads clearly less than the playhead itself wherever either is drawn
    // (the ruler for their heads, their solid halo for their lines); a
    // scroll bar's handle under the pointer or held reads more than at rest;
    // and the ramp's hover and on fills stay apart.
    void quietAndActiveStatesStayApart() {
        const EditorTheme t = makeEditorTheme();
        QVERIFY(t.playheadIdle.hsvSaturation() < 70);
        QCOMPARE(t.playheadHalo.alpha(), 255);
        for (const QColor &surface : {t.ruler, t.rulerBottom, t.playheadHalo})
            QVERIFY2(contrastRatio(t.playheadIdle, surface) + 0.35 <= contrastRatio(t.playhead, surface),
                     qPrintable(QStringLiteral("%1 and %2 on %3").arg(t.playheadIdle.name(), t.playhead.name(), surface.name())));
        // The playhead is the design's red, as it is, stroked in the stage's dark.
        QCOMPARE(t.playhead.rgb(), tokens::color(tokens::playhead).rgb());
        QCOMPARE(t.playheadHalo.rgb(), tokens::color(tokens::surround).rgb());
        const LauncherTheme l = launcherTheme();
        const auto stronger = [](const QColor &rest, const QColor &active, const QColor &surface) {
            return contrastRatio(laidOn(surface, active), surface) > contrastRatio(laidOn(surface, rest), surface) + 1.0;
        };
        for (const QColor &surface : {t.panel, t.well, t.field, t.dialog})
            QVERIFY2(stronger(t.scrollHandle, t.scrollHandleHover, surface), qPrintable(surface.name()));
        for (const QColor &surface : {l.content, l.group, l.sidebar})
            QVERIFY2(stronger(l.scrollHandle, l.scrollHandleHover, surface), qPrintable(surface.name()));
        // Hover and on stay apart from rest and from each other: raised under
        // the pointer, on when chosen, hover2 over a raised surface.
        QVERIFY(t.raised.lightness() < t.on.lightness() && t.on.lightness() < t.hover2.lightness());
        QVERIFY(t.chrome.lightness() < t.raised.lightness());
        // Both sheets give the stronger handle to a held one as well as a hovered one.
        QVERIFY(editorStyleSheet(editorTheme()).contains(QStringLiteral("QScrollBar::handle:vertical:pressed")));
        QVERIFY(launcherStyleSheet(launcherTheme()).contains(QStringLiteral("QScrollBar::handle:vertical:pressed")));
    }

    // The editor draws the design's graphite ramp (mockup-spec 0.2) and
    // nothing else: exact surfaces, the flat bars, menus, dialogs and the
    // states. With KERF_SCREENSHOT_DIR set, each picture is saved.
    void editorRendersTheGraphiteRamp() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QSignalSpy changed(EditorAppearance::instance(), &EditorAppearance::changed);
        const QJsonObject preset{{"name", "Harbor Intro"}, {"document", "kerf 1\nproject name=\"Harbor Intro\"\n"}, {"sequence", "s1"},
            {"slots", QJsonArray{
                QJsonObject{{"id", "Opening"}, {"seconds", 4}, {"video", true}, {"audio", true},
                    {"clips", QJsonArray{QJsonObject{{"id", "v1a"}, {"name", "Wide"}, {"sequence", "s1"}, {"track", "V1"}, {"at", "00:00:00:00"}}}},
                    {"fits", QJsonArray{QJsonObject{{"media", "wide"}, {"latestFrame", 380}}, QJsonObject{{"media", "close"}, {"latestFrame", 380}}}}},
                QJsonObject{{"id", "Logo"}, {"seconds", 2}, {"video", true}, {"audio", false},
                    {"clips", QJsonArray{QJsonObject{{"id", "t1"}, {"name", "Logo"}, {"sequence", "s1"}, {"track", "V2"}, {"at", "00:00:01:00"}}}},
                    {"fits", QJsonArray{QJsonObject{{"media", "drone"}, {"latestFrame", 432}}}}}}},
            {"textSlots", QJsonObject{{"title1", "Harbor Teaser"}}}};

        {
            const QString name = QStringLiteral("dark");
            // Each picture starts from the Editing layout.
            QSettings().remove(QStringLiteral("workspace"));
            // The one theme, whatever the system says.
            const EditorTheme &theme = editorTheme();
            // The ramp, exactly (mockup-spec 0.2).
            const std::pair<const char *, QColor> ramp[] = {
                {"#050505", theme.window},    {"#0e0e0e", theme.viewerSurround}, {"#121212", theme.well},
                {"#171717", theme.chrome},    {"#171717", theme.panel},
                {"#1c1c1c", theme.card},      {"#171717", theme.chatSurface},    {"#1c1c1c", theme.menu},
                {"#1c1c1c", theme.tooltip},   {"#1c1c1c", theme.dialog},         {"#262626", theme.raised},
                {"#262626", theme.field},     {"#303030", theme.on},             {"#262626", theme.toolSelected},
                {"#363636", theme.hover2},    {"#ededed", theme.text},           {"#a8a8a8", theme.textSecondary},
                {"#828282", theme.ink3},      {"#5c5c5c", theme.textDisabled},   {"#3a3a3a", theme.track},
                {"#6a6a6a", theme.track2},    {"#3366e6", theme.accent},         {"#2b5ad6", theme.accentHover},
                {"#8fb0ff", theme.accentInk}, {"#4cc38a", theme.success},        {"#ec5d61", theme.error},
                {"#e5484d", theme.playhead},  {"#d6a345", theme.warning},
            };
            for (const auto &[code, color] : ramp)
                QVERIFY2(color.rgba() == QColor(code).rgba(), qPrintable(QStringLiteral("%1 for %2").arg(color.name(), code)));
            QCOMPARE(theme.line.rgba(), qRgba(255, 255, 255, 20));
            QCOMPARE(theme.line2.rgba(), qRgba(255, 255, 255, 36));
            QCOMPARE(theme.lineh.rgba(), qRgba(255, 255, 255, 56));
            // Every surface is neutral graphite.
            for (const QColor &surface : {theme.window, theme.panel, theme.chrome, theme.well, theme.field, theme.dialog,
                                          theme.chatSurface, theme.viewerSurround, theme.raised, theme.on})
                QVERIFY2(isGraphite(surface), qPrintable(surface.name()));
            // Every token is filled in, and nothing amber or gold is left but
            // the design's own warn colour.
            const QString sheet = qApp->styleSheet();
            QVERIFY(!sheet.contains(QLatin1Char('@')));
            static const QRegularExpression hex(QStringLiteral("#[0-9a-fA-F]{6,8}\\b"));
            for (auto it = hex.globalMatch(sheet); it.hasNext();) {
                const QString code = it.next().captured();
                QVERIFY2(!isAmber(QColor(code)) || QColor(code).rgb() == theme.warning.rgb(), qPrintable(code));
            }
            // No gradients and no bevels: every bezel is flat.
            QVERIFY(!sheet.contains(QStringLiteral("qlineargradient")));
            QVERIFY(!sheet.contains(QStringLiteral("border-top-color")));
            // The application palette is the ramp, its accent Kerf's.
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::Base), theme.field);
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::Window), theme.chrome);
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::Accent), theme.accent);
#endif
            // Clip labels read on their headers, at the base of the name bar
            // and at its lightened top, where the label sits.
            for (const ClipKind kind : {ClipKind::Video, ClipKind::Audio, ClipKind::Music, ClipKind::Generator, ClipKind::Caption}) {
                QVERIFY(contrastRatio(theme.clip(kind).label, theme.clip(kind).header) >= 4.5);
                QVERIFY(contrastRatio(theme.clip(kind).label, theme.clip(kind).header.lighter(108)) >= 4.5);
            }
            // Every marker reads as a mark on the ruler.
            for (const char *marker : {"red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white"})
                QVERIFY2(contrastRatio(theme.marker(QString::fromLatin1(marker)), theme.ruler) >= 3.0, marker);
            // Secondary text reads on the window and dialogs too, and Kerf's
            // text on the proposal card.
            QVERIFY(contrastRatio(theme.textSecondary, theme.dialog) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.panelTitle) >= 4.5);
            QVERIFY(contrastRatio(theme.assistantText, theme.assistantSurface) >= 4.5);
            QVERIFY(contrastRatio(theme.assistantText, theme.assistantHover) >= 4.5);
            QVERIFY(contrastRatio(theme.badgeErrorText, theme.badgeError) >= 4.5);
            // A title strip is chrome, whichever tab is current: names, not boxes.
            QCOMPARE(theme.panelTitleActive.rgb(), theme.panelTitle.rgb());
            // The accent as text reads on each surface it is drawn on; see
            // accentTextReadsOnEverySurfaceItIsDrawnOn for every accent.
            for (const QColor &surface : {theme.panel, theme.rulerCorner})
                QVERIFY2(contrastRatio(theme.accentText, surface) >= 4.5, qPrintable(surface.name()));
            // An unchecked check box's edge is the design's outline.
            const auto over = [](const QColor &under, QColor top) {
                const qreal alpha = top.alphaF();
                top.setAlpha(255);
                return mixColor(under, top, alpha);
            };
            // (The design's check box outline is white at 16%, a hairline,
            // not a 3:1 edge; its checked state is the on fill.)
            QCOMPARE(theme.checkBorder.rgb(), over(theme.chrome, tokens::color(tokens::checkboxOutline)).rgb());
            QVERIFY(contrastRatio(theme.assistantText, theme.chrome) >= 4.5);
            QVERIFY(contrastRatio(theme.tabInactiveText, theme.panelTitle) >= 4.5);
            QVERIFY(contrastRatio(theme.tileEmptyText, theme.tileEmpty) >= 4.5);
            QVERIFY(contrastRatio(theme.cardPlaceholderText, theme.cardPlaceholder) >= 4.5);
            QVERIFY(contrastRatio(theme.badgeInfoText, theme.badgeInfo) >= 4.5);
            // The empty viewer's hint is on the black card; the Source
            // monitor's message is on the stage, which follows the appearance.
            QVERIFY(contrastRatio(theme.viewerTextDim, theme.viewerCanvas) >= 4.5);
            QVERIFY(contrastRatio(theme.viewerText, theme.viewerCanvas) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.viewerSurround) >= 4.5);
            QVERIFY(contrastRatio(theme.textSecondary, theme.panel) >= 4.5);
            // Placeholders carry guidance (example searches, what to ask), so
            // they read as text.
            QVERIFY(contrastRatio(theme.placeholderText, theme.field) >= 4.5);
            QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::PlaceholderText), theme.placeholderText);
            // The Source monitor's marked range is information: 3:1 on its panel.
            QVERIFY(contrastRatio(theme.inOutRange, theme.panel) >= 3.0);
            // The changed-on-disk triangle reads on its bar.
            QVERIFY(contrastRatio(theme.warningGlyph, theme.warningSurface) >= 3.0);
            // A control focused from the keyboard is outlined in the solid accent.
            QCOMPARE(theme.focusHalo.rgba(), theme.accent.rgba());
            // The stage is the surround, the picture's canvas black.
            QCOMPARE(theme.viewerSurround.rgb(), QColor("#0e0e0e").rgb());
            QCOMPARE(theme.viewerCanvas.rgb(), QColor("#000000").rgb());

            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(showcaseSnapshot(12, QStringLiteral("clean")));
            window.selectClip(QStringLiteral("v1b"));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
            window.handleEvent(QJsonObject{{"type", "preset_library"}, {"directory", "presets"}, {"presets", QJsonArray{
                QJsonObject{{"path", "presets/harbor-intro.kerfpreset"}, {"name", "Harbor Intro"}, {"author", "jun"},
                            {"updated", "2026-10-30T09:00:00Z"}, {"slots", QJsonArray{
                                QJsonObject{{"id", "Opening"}, {"seconds", 4}, {"video", true}, {"audio", true}, {"clips", 2}},
                                QJsonObject{{"id", "Logo"}, {"seconds", 2}, {"video", true}, {"audio", false}, {"clips", 1}}}},
                            {"texts", QJsonObject{{"title1", "Title"}}}},
                QJsonObject{{"path", "presets/lower-thirds.kerfpreset"}, {"name", "Lower Thirds"}, {"author", "mina"},
                            {"updated", "2026-10-28T15:30:00Z"}, {"slots", QJsonArray{}}, {"texts", QJsonObject{{"name", "Name"}, {"role", "Role"}}}},
                QJsonObject{{"path", "presets/old.kerfpreset"}, {"name", "old"}, {"error", "line 3: unknown statement"}}}}});
            fakeArtwork(window.m_artwork);
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // Test builds have no runtime to send to; that is not this picture's subject.
            window.m_errorBanner->hide();
            // Nothing under the pointer.
            QTest::mouseMove(window.m_toolbar, QPoint(window.m_toolbar->width() / 2, 4));
            QVERIFY(window.m_timeline->hasProposal());
            QCoreApplication::processEvents();
            const QImage image = window.grab().toImage();
            // The bars are the theme's: the toolbar's plate (flat chrome).
            auto *toolbar = window.findChild<QToolBar *>();
            QVERIFY(qobject_cast<FaceplateToolBar *>(toolbar));
            {
                const QImage plate = toolbar->grab().toImage();
                const qreal ratio = plate.devicePixelRatio();
                // Its 1px edge along the foot, and the plate above it.
                QCOMPARE(plate.pixelColor(QPoint(1, toolbar->height() - 1) * ratio).rgb(),
                         laidOn(tokens::color(tokens::chrome), tokens::color(tokens::line)).rgb());
                const QColor middle = plate.pixelColor(QPoint(1, toolbar->height() / 2) * ratio);
                QVERIFY2(nearColor(middle.rgb(), tokens::color(tokens::chrome).rgb()), qPrintable(middle.name()));
            }
            // The window is titled with the project, so the Window menu,
            // Mission Control and the Dock name it, and the toolbar shows the same.
            QCOMPARE(window.windowTitle(), QStringLiteral("Harbor Teaser"));
            QCOMPARE(window.m_projectTitle->text(), window.windowTitle());
            // The title is UI 13, centred on the window (the toolbar spans
            // it), clear of the groups either side.
            QLabel *title = window.m_projectTitle;
            QCOMPARE(title->font().pixelSize(), 13);
            QVERIFY2(std::abs(title->geometry().center().x() - toolbar->width() / 2) <= 1,
                     qPrintable(QStringLiteral("%1 of %2").arg(title->geometry().center().x()).arg(toolbar->width())));
            QVERIFY(title->geometry().right() < window.m_formatBank->geometry().left());
            // The flat bar: the format bank, 12, Export as tall as the bank's
            // keys, 12 in from the window's right edge.
            ExportButton *exportButton = window.m_exportButton;
            const auto inBar = [toolbar](const QWidget *item) { return QRect(item->mapTo(toolbar, QPoint(0, 0)), item->size()); };
            QCOMPARE(inBar(exportButton).left() - inBar(window.m_formatBank).right() - 1, 12);
            QCOMPARE(exportButton->height(), window.m_formatBank->height());
            QCOMPARE(toolbar->width() - inBar(exportButton).right() - 1, 12);
            // The toolbar is chrome.
            QCOMPARE(dominantColor(toolbar->grab().toImage()), theme.chrome.rgb());
            // The tools live in the timeline bar's segmented control; the
            // old vertical palette is gone.
            QVERIFY(!window.findChild<QToolBar *>(QStringLiteral("toolPalette")));
            // An unavailable command's symbol is dimmed (the kit's `dis`), never
            // brightened: no pixel of the strip's transport is lighter.
            {
                window.setConnection(QStringLiteral("disconnected"));
                auto *button = window.findChild<ui::IconButton *>(QStringLiteral("timelineGoToEnd"));
                QVERIFY(button && !button->isEnabled());
                QCOMPARE(button->iconColor().rgb(), tokens::dis);
                const QImage dim = button->grab().toImage();
                int lightest = 0;
                for (int y = 0; y < dim.height(); ++y)
                    for (int x = 0; x < dim.width(); ++x) lightest = std::max(lightest, qGray(dim.pixel(x, y)));
                QVERIFY2(lightest <= qGray(tokens::dis) + 1, qPrintable(QString::number(lightest)));
                window.setConnection(QStringLiteral("ready"));
            }
            save(image, QStringLiteral("editor-%1").arg(name));
            // Export's menu opens as the design's menu: a card, rounded and
            // clipped to its corners, under the button.
            window.m_exportButton->showMenu();
            QMenu *exportMenu = window.m_exportButton->menu();
            QVERIFY(QTest::qWaitForWindowExposed(exportMenu));
            QVERIFY(exportMenu->testAttribute(Qt::WA_TranslucentBackground));
            {
                const QImage popup = exportMenu->grab().toImage();
                save(popup, QStringLiteral("popup-%1").arg(name));
                QCOMPARE(popup.pixelColor(0, 0).alpha(), 0);
                QCOMPARE(amberPixels(popup), 0);
                // The card (inside the translucent shadow margin) opens under the button.
                const QMargins shadow = ui::shadowMargins();
                const int cardTop = exportMenu->geometry().top() + std::max(shadow.top(), shadow.bottom());
                QVERIFY2(cardTop > window.m_exportButton->mapToGlobal(QPoint(0, window.m_exportButton->height() - 1)).y(),
                         qPrintable(QString::number(cardTop)));
            }
            exportMenu->hide();
            // So does a clip's context menu: rounded and clipped to its
            // corners, the row under the pointer a rounded `raised` fill,
            // every row 28pt.
            {
                QImage shape;
                QRect row;
                QList<QRect> others;
                bool translucent = false;
                bool symbols = false;
                bool iconColumn = false;
                // Captured while it is open, and closed whatever happens.
                QTimer::singleShot(0, &window, [&] {
                    auto *menu = qobject_cast<QMenu *>(QApplication::activePopupWidget());
                    if (!menu) return;
                    const auto close = qScopeGuard([menu] { menu->close(); });
                    for (QAction *action : menu->actions()) {
                        symbols = symbols || !action->icon().isNull();
                        if (!action->isEnabled() || action->isSeparator()) continue;
                        if (row.isValid()) {
                            others << menu->actionGeometry(action);
                            continue;
                        }
                        menu->setActiveAction(action);
                        row = menu->actionGeometry(action);
                    }
                    translucent = menu->testAttribute(Qt::WA_TranslucentBackground);
                    iconColumn = menu->property("iconColumn").toBool();
                    shape = menu->grab().toImage();
                });
                QTimer::singleShot(2000, &window, [] {
                    if (QWidget *popup = QApplication::activePopupWidget()) popup->close();
                });
                emit window.m_timeline->clipMenuRequested(QStringLiteral("v1b"), window.mapToGlobal(QPoint(400, 300)));
                QVERIFY(!shape.isNull() && row.isValid() && !others.isEmpty());
                save(shape, QStringLiteral("menu-%1").arg(name));
                QVERIFY(translucent);
                const qreal ratio = shape.devicePixelRatio();
                QCOMPARE(shape.pixelColor(0, 0).alpha(), 0);
                QCOMPARE(shape.pixelColor(QPoint(row.left() + 2, row.center().y()) * ratio).rgb(), theme.menuSelection.rgb());
                QCOMPARE(row.height(), 28);
                QCOMPARE(amberPixels(shape), 0);
                // Menus show no symbols, as macOS's, though Split and Delete
                // have them for the toolbar: each row's first 20px are its
                // fill, the highlighted row's too, and its text starts 22px
                // in, as in a menu of commands without symbols.
                QVERIFY(QCoreApplication::testAttribute(Qt::AA_DontShowIconsInMenus));
                // (The registry's actions carry no symbols now; any that
                // did would keep the icon column, shown empty.)
                QVERIFY(!symbols || iconColumn);
                others.prepend(row);
                for (const QRect &item : std::as_const(others)) {
                    const QColor fill = item == row ? theme.menuSelection : theme.menu;
                    for (int y = item.center().y() - 4; y <= item.center().y() + 4; ++y)
                        for (int x = item.left() + 1; x <= item.left() + 20; ++x)
                            QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), fill.rgb());
                    if (item != row)
                        QVERIFY(highestContrast(shape, QRect(QPoint(item.left() + 21, item.top()) * ratio, QSize(5, item.height()) * ratio),
                                                fill) >= 3.0);
                }
                // Nor do the menu bar's menus, native on macOS.
                for (QAction *menuAction : window.menuBar()->actions())
                    if (QMenu *menu = menuAction->menu())
                        for (QAction *action : menu->actions()) QVERIFY2(!action->isIconVisibleInMenu(), qPrintable(action->text()));
            }
            // A menu with checks (Collaborate's Live Sync) pads its rows by
            // less, so its text starts where a menu's without checks does.
            {
                QMenu *panels = nullptr;
                for (QAction *action : window.menuBar()->actions())
                    if (action->text() == QStringLiteral("Collaborate")) panels = action->menu();
                QVERIFY(panels);
                {
                    QAction *live = window.m_commands->action(QStringLiteral("collab.live"));
                    const QSignalBlocker blocker(live);
                    live->setChecked(true);
                }
                panels->popup(window.mapToGlobal(QPoint(400, 300)));
                QVERIFY(QTest::qWaitForWindowExposed(panels));
                QVERIFY(panels->property("checkColumn").toBool());
                const QImage shape = panels->grab().toImage();
                save(shape, QStringLiteral("menu-checks-%1").arg(name));
                const qreal ratio = shape.devicePixelRatio();
                QCOMPARE(shape.pixelColor(0, 0).alpha(), 0);
                QCOMPARE(amberPixels(shape), 0);
                // Its checks are the pop-ups' thin check, 7px into the row,
                // not Fusion's tick at the row's start.
                QAction *checked = nullptr;
                QAction *submenu = nullptr;
                for (QAction *action : panels->actions()) {
                    if (!checked && action->isChecked()) checked = action;
                    if (!submenu && action->menu()) submenu = action;
                }
                QVERIFY(checked && submenu);
                const QRect check = panels->actionGeometry(checked);
                for (int y = check.center().y() - 4; y <= check.center().y() + 4; ++y)
                    for (int x = check.left(); x <= check.left() + 4; ++x)
                        QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), theme.menu.rgb());
                QVERIFY(highestContrast(shape, QRect(QPoint(check.left() + 6, check.center().y() - 5) * ratio, QSize(11, 11) * ratio),
                                        theme.menu) >= 3.0);
                // A submenu's chevron is thin and open, not Fusion's filled
                // triangle: inside its mouth is the row's fill.
                const QRect arrow = panels->actionGeometry(submenu);
                for (int y = arrow.center().y() - 1; y <= arrow.center().y() + 1; ++y)
                    for (int x = arrow.right() - 13; x <= arrow.right() - 12; ++x)
                        QCOMPARE(shape.pixelColor(QPoint(x, y) * ratio).rgb(), theme.menu.rgb());
                QVERIFY(highestContrast(shape, QRect(QPoint(arrow.right() - 14, arrow.center().y() - 5) * ratio, QSize(7, 11) * ratio),
                                        theme.menu) >= 3.0);
                panels->close();
                QVERIFY(!window.binMenu(QString())->property("checkColumn").toBool());
            }

            // Settings.
            window.m_preferences->show();
            QVERIFY(QTest::qWaitForWindowExposed(window.m_preferences));
            QCoreApplication::processEvents();
            const QImage settings = window.m_preferences->grab().toImage();
            QCOMPARE(settings.pixelColor(QPoint(4, settings.height() - 4)).rgb(), theme.dialog.rgb());
            // Sized to what it shows, with no Close button under empty space.
            QVERIFY(window.m_preferences->height() < 400);
            QVERIFY(!window.m_preferences->findChild<QDialogButtonBox *>());
            save(settings, QStringLiteral("settings-%1").arg(name));
            // Advanced opens below, and the window grows to show it.
            auto *advanced = window.m_preferences->findChild<QToolButton *>(QStringLiteral("disclosure"));
            QVERIFY(advanced);
            // Its chevron, the only sign of whether it is open, reads on the
            // dialog (3:1), closed and open.
            QCOMPARE(ink(advanced->icon()), theme.textSecondary.rgb());
            const int closedHeight = window.m_preferences->height();
            advanced->click();
            QVERIFY(window.m_advancedSettings->isVisible());
            QCOMPARE(ink(advanced->icon()), theme.textSecondary.rgb());
            QVERIFY(window.m_preferences->height() > closedHeight);
            advanced->click();
            window.m_preferences->hide();

            // A representative dialog: the preset's media bindings.
            QTimer::singleShot(0, &window, [&] {
                auto *dialog = window.findChild<QDialog *>(QStringLiteral("presetBindingsDialog"));
                QVERIFY(dialog);
                QTimer::singleShot(2000, dialog, &QDialog::reject);
                for (const char *slot : {"presetMedia_Opening", "presetMedia_Logo"})
                    if (auto *choice = dialog->findChild<QComboBox *>(QString::fromLatin1(slot))) choice->setCurrentIndex(1);
                QCoreApplication::processEvents();
                const QImage bindings = dialog->grab().toImage();
                QCOMPARE(dominantColor(bindings), theme.dialog.rgb());
                // The default button is the accent, as in macOS dialogs.
                QPushButton *create = dialog->findChild<QDialogButtonBox *>()->button(QDialogButtonBox::Ok);
                QCOMPARE(create->text(), QStringLiteral("Create Proposal"));
                // Its row of pop-up and seconds field is one 24pt line, as the
                // design's controls are.
                auto *choice = dialog->findChild<QComboBox *>(QStringLiteral("presetMedia_Opening"));
                auto *from = dialog->findChild<QDoubleSpinBox *>(QStringLiteral("presetFrom_Opening"));
                QVERIFY(choice && from);
                QCOMPARE(choice->height(), 24);
                QCOMPARE(from->height(), 24);
                QCOMPARE(create->height(), 24);
                QCOMPARE(from->geometry().center().y(), choice->geometry().center().y());
                QVERIFY(popupShowsEveryRow(choice));
                // Its pop-ups are buttons' bezels, as the launcher's Frame
                // Rate and the toolbar's pickers, not fields like Text 1.
                QVERIFY(popUpHasButtonBezel(bindings, choice, theme.button));
                QVERIFY(create->isEnabled() && create->isDefault());
                const QImage createImage = create->grab().toImage();
                const QColor createFill = createImage.pixelColor(QPoint(6, create->height() / 2) * createImage.devicePixelRatio());
                QVERIFY2(std::abs(createFill.hue() - theme.accent.hue()) <= 4, qPrintable(createFill.name()));
                // Sized to its slots, as macOS dialogs are, with no empty
                // window under them; Details grows it, and closing shrinks it back.
                QVERIFY2(dialog->height() < 400, qPrintable(QString::number(dialog->height())));
                save(bindings, QStringLiteral("dialog-%1").arg(name));
                auto *details = dialog->findChild<QToolButton *>(QStringLiteral("disclosure"));
                QVERIFY(details);
                QCOMPARE(ink(details->icon()), theme.textSecondary.rgb());
                const int closedHeight = dialog->height();
                details->click();
                QCOMPARE(ink(details->icon()), theme.textSecondary.rgb());
                QVERIFY(dialog->height() > closedHeight);
                details->click();
                QCOMPARE(dialog->height(), closedHeight);
                dialog->reject();
            });
            window.showPresetBindings(QStringLiteral("presets/harbor-intro.kerfpreset"), preset);

            // States: project.kerf changed on disk, the agent's proposal and
            // next steps, and a conversation in the assistant panel.
            // As kerfd sends it, so the review model hears the proposals too.
            window.handleEvent(QJsonObject{{"type", "state"}, {"snapshot", showcaseSnapshot(13, QStringLiteral("diverged"))}});
            showAssistant(window);
            window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
            window.appendConversation(QStringLiteral("Kerf"),
                QStringLiteral("I trimmed the close-up by 1.5 seconds, removed the drone pass, and added a lower third at 12 s."));
            window.appendConversation(QStringLiteral("Error"), QStringLiteral("The music cue could not be found on disk."));
            window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b · removed v1c"));
            window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                            QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music under the dialogue"}}});
            QCoreApplication::processEvents();
            window.m_errorBanner->hide();
            QCoreApplication::processEvents();
            QVERIFY(!window.m_divergedBanner->isHidden());
            // The pinned proposal's card (mockup .pcard): Reject, then the
            // primary Accept filled with the accent, its label reading on it.
            ProposalCard *pinned = nullptr;
            for (ProposalCard *card : window.m_chat->cards())
                if (card->isPinned()) pinned = card;
            QVERIFY(pinned);
            ui::Button *accept = pinned->acceptButton();
            // In the conversation watching comes first; on its own, Accept.
            QCOMPARE(accept->kind(), ui::Button::Kind::Secondary);
            pinned->setCompact(false);
            pinned->setOpen(true);
            QCoreApplication::processEvents();
            QCOMPARE(accept->kind(), ui::Button::Kind::Primary);
            QCOMPARE(pinned->rejectButton()->parentWidget(), accept->parentWidget());
            QVERIFY(pinned->rejectButton()->geometry().right() < accept->geometry().left());
            {
                const QImage pill = accept->grab().toImage();
                QCOMPARE(pill.pixelColor(QPoint(4, accept->height() / 2) * pill.devicePixelRatio()).rgb(), tokens::accent);
                QVERIFY(contrastRatio(QColor(tokens::accentText), QColor(tokens::accent)) >= 4.5);
            }
            const QImage states = window.grab().toImage();
            // The Script's diff: removals in neg, additions in pos.
            QVERIFY(!pinned->diffLines().isEmpty());
            QVERIFY(pinned->diffView()->text().contains(tokens::color(tokens::neg).name()));
            QVERIFY(pinned->diffView()->text().contains(tokens::color(tokens::pos).name()));
            // Kerf's next steps are the welcome row's suggestions.
            QVERIFY(!window.m_chat->welcomeRow()->suggestions().isEmpty());
            // The banner's buttons, as macOS orders them: Load from Disk last,
            // 8 past the button before it, 8 in from the window's edge.
            QCOMPARE(window.m_loadFromDisk->x() - window.m_keepEditorVersion->geometry().right() - 1, 8);
            QCOMPARE(window.m_loadFromDisk->mapTo(&window, QPoint(window.m_loadFromDisk->width(), 0)).x(), window.width() - 8);
            // The bar and its text are neutral; only the 14-point triangle is orange.
            for (QWidget *part : window.m_divergedBanner->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
                if (part != window.m_divergedGlyph) QCOMPARE(amberPixels(part->grab().toImage()), 0);
            QCOMPARE(dominantColor(window.m_divergedBanner->grab().toImage()), theme.warningSurface.rgb());
            // The banner is its own bar, not a second row of the toolbar: a
            // step lighter than it.
            QVERIFY(theme.warningSurface.rgb() != theme.chrome.rgb());
            QVERIFY(theme.warningSurface.lightness() > theme.chrome.lightness());
            save(states, QStringLiteral("states-%1").arg(name));

            // Scroll bars are hidden, as the design hides them: shown, they
            // take no room, so nothing is framed where two would meet.
            {
                QScrollArea *scroll = window.m_timelineScroll;
                const Qt::ScrollBarPolicy verticalPolicy = scroll->verticalScrollBarPolicy();
                const Qt::ScrollBarPolicy horizontalPolicy = scroll->horizontalScrollBarPolicy();
                scroll->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOn);
                scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOn);
                QCoreApplication::processEvents();
                QCOMPARE(scroll->verticalScrollBar()->width(), 0);
                QCOMPARE(scroll->horizontalScrollBar()->height(), 0);
                scroll->setVerticalScrollBarPolicy(verticalPolicy);
                scroll->setHorizontalScrollBarPolicy(horizontalPolicy);
            }

            window.close();
        }
        // The theme was applied once, before this test; nothing above asked
        // for another, so nothing changed.
        QCOMPARE(changed.count(), 0);

        // An open window keeps the one theme when the old scheme API is
        // called: no change is signalled, and its docks, the conversation's
        // panel and the comments it shows draw the same.
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("Done."));
        QCoreApplication::processEvents();
        const auto surface = [&window] {
            // Clear over the window's room.
            const QImage panel = window.m_shell->grab().toImage();
            return panel.pixelColor(QPoint(window.m_chat->x() + 4, window.m_chat->y() + window.m_chat->height() / 2) * panel.devicePixelRatio()).rgb();
        };
        const auto nameInk = [&window] {
            QLabel *name = window.m_chat->rows().constFirst()->nameLabel();
            name->ensurePolished();
            return name->palette().color(QPalette::WindowText).rgb();
        };
        QCOMPARE(surface(), editorTheme().chatSurface.rgb());
        QCOMPARE(nameInk(), editorTheme().chatText.rgb());
        for (int again = 0; again < 3; ++again) applyEditorAppearance();
        QCOMPARE(changed.count(), 0);
        QCoreApplication::processEvents();
        QCOMPARE(surface(), editorTheme().chatSurface.rgb());
        QCOMPARE(nameInk(), editorTheme().chatText.rgb());
        QVERIFY(!window.m_disclosures.isEmpty());
        for (QToolButton *disclosure : std::as_const(window.m_disclosures))
            QCOMPARE(ink(disclosure->icon()), editorTheme().textSecondary.rgb());
    }

    // Push buttons and pop-up buttons take focus from Tab alone, so a click
    // leaves no accent behind. Tab outlines the control as the design does
    // (2pt of the solid accent, 1pt outside its bezel), and the outline
    // moves with the focus. A field being typed in is outlined however it
    // got focus.
    void focusRingsFollowTheKeyboardAndTyping() {
        {
            const EditorTheme &theme = editorTheme();
            QDialog dialog;
            auto *layout = new QHBoxLayout(&dialog);
            layout->setContentsMargins(12, 12, 12, 12);
            auto *field = new QLineEdit;
            auto *reject = new QPushButton(QStringLiteral("Reject"));
            auto *choice = new QComboBox;
            choice->addItems({QStringLiteral("Preview"), QStringLiteral("Full")});
            auto *seconds = new StepperSpinBox;
            auto *details = new QPushButton(QStringLiteral("Details"));
            layout->addWidget(reject);
            layout->addWidget(field);
            layout->addWidget(choice);
            layout->addWidget(seconds);
            layout->addWidget(details);
            dialog.show();
            QVERIFY(QTest::qWaitForWindowActive(&dialog));
            QCOMPARE(reject->focusPolicy(), Qt::TabFocus);
            QCOMPARE(choice->focusPolicy(), Qt::TabFocus);
            // The focus a window gives its first control as it opens is no
            // keyboard navigation, so that button is not ringed.
            QCOMPARE(QApplication::focusWidget(), reject);
            QVERIFY(!dialog.findChild<QWidget *>(QStringLiteral("focusHalo")));
            const auto ringAround = [&](QWidget *control) {
                QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
                const auto halos = dialog.findChildren<QWidget *>(QStringLiteral("focusHalo"));
                QCOMPARE(halos.size(), 1);
                QVERIFY(halos.constFirst()->isVisible());
                QCOMPARE(halos.constFirst()->geometry(), control->geometry().adjusted(-3, -3, 3, 3));
                // The outline, 2pt out from the bezel, is the accent.
                const QImage image = dialog.grab().toImage();
                const QColor ring = image.pixelColor(QPoint(control->x() - 2, control->geometry().center().y()) * image.devicePixelRatio());
                const QColor expected = theme.accent;
                QVERIFY2(std::abs(ring.red() - expected.red()) <= 3 && std::abs(ring.green() - expected.green()) <= 3 &&
                             std::abs(ring.blue() - expected.blue()) <= 3,
                         qPrintable(ring.name() + QStringLiteral(" for ") + expected.name()));
            };
            // A click puts the caret in the field, which is ringed.
            QTest::mouseClick(field, Qt::LeftButton);
            QVERIFY(field->hasFocus());
            ringAround(field);
            // A click on a button takes no focus, so the field keeps it.
            QTest::mouseClick(reject, Qt::LeftButton);
            QVERIFY(!reject->hasFocus());
            ringAround(field);
            // A pop-up is a button's bezel, not a field's.
            const QImage unfocused = dialog.grab().toImage();
            QVERIFY(popUpHasButtonBezel(unfocused, choice, theme.button));
            // Tab rings the pop-up, the stepper's field, then the buttons.
            QTest::keyClick(field, Qt::Key_Tab);
            QVERIFY(choice->hasFocus());
            ringAround(choice);
            // The ring alone shows a pop-up's focus: its edge stays as it was.
            {
                const QImage image = dialog.grab().toImage();
                const QPoint edge = QPoint(choice->x(), choice->geometry().center().y()) * image.devicePixelRatio();
                QCOMPARE(image.pixelColor(edge).rgb(), unfocused.pixelColor(edge).rgb());
            }
            QTest::keyClick(choice, Qt::Key_Tab);
            QVERIFY(seconds->hasFocus());
            ringAround(seconds);
            QTest::keyClick(seconds, Qt::Key_Tab);
            QVERIFY(details->hasFocus());
            ringAround(details);
            QTest::keyClick(details, Qt::Key_Tab);
            QVERIFY(reject->hasFocus());
            ringAround(reject);
            // Back to the field: the ring moves to it.
            field->setFocus();
            ringAround(field);
            // A multi-line field is ringed as it is typed in too, around its
            // 3px bezel; a read-only one, which takes no typing, is not.
            auto *notes = new QPlainTextEdit;
            auto *transcript = new QTextEdit;
            transcript->setReadOnly(true);
            layout->addWidget(notes);
            layout->addWidget(transcript);
            notes->show();
            transcript->show();
            QCoreApplication::processEvents();
            QTest::mouseClick(notes->viewport(), Qt::LeftButton);
            QVERIFY(notes->hasFocus());
            ringAround(notes);
            QCOMPARE(notes->property("bezelRadius").toInt(), 3);
            QTest::mouseClick(transcript->viewport(), Qt::LeftButton);
            QVERIFY(transcript->hasFocus());
            QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
            QVERIFY(!dialog.findChild<QWidget *>(QStringLiteral("focusHalo")));
            // A combo box that takes typing stays a field: a line edit on
            // the field's fill (the flat ramp gives fields and secondary
            // buttons the same `raised`), with no chevron cell of its own.
            auto *typed = new QComboBox;
            typed->setEditable(true);
            typed->addItem(QStringLiteral("Kerf Cloud"));
            layout->addWidget(typed);
            typed->show();
            QCoreApplication::processEvents();
            QVERIFY(typed->lineEdit());
            const QImage shot = dialog.grab().toImage();
            QCOMPARE(shot.pixelColor(QPoint(typed->x() + 4, typed->geometry().center().y()) * shot.devicePixelRatio()).rgb(),
                     theme.field.rgb());
        }
    }

    // The flat style sheet keeps the design's control metrics (mockup-spec
    // 0.4, 0.5): buttons, fields and pop-ups 24pt with 6pt corners, a dialog
    // button at least 64pt wide, 14pt check boxes, a 16pt slider with a 10pt
    // knob, 28pt menu rows, a 24pt tooltip on a rounded card, and scroll bars
    // that take no room.
    void flatControlsKeepTheDesignsMetrics() {
        QDialog dialog;
        auto *layout = new QVBoxLayout(&dialog);
        auto *button = new QPushButton(QStringLiteral("OK"));
        auto *field = new QLineEdit;
        auto *choice = new QComboBox;
        choice->addItems({QStringLiteral("Master"), QStringLiteral("Draft")});
        auto *seconds = new StepperSpinBox;
        auto *box = new QCheckBox(QStringLiteral("Include comments as EDL locators"));
        auto *slider = new QSlider(Qt::Horizontal);
        auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
        auto *list = new QListWidget;
        for (int row = 0; row < 40; ++row) list->addItem(QStringLiteral("Row %1").arg(row));
        for (QWidget *widget : std::initializer_list<QWidget *>{button, field, choice, seconds, box, slider, buttons, list})
            layout->addWidget(widget);
        dialog.show();
        QVERIFY(QTest::qWaitForWindowExposed(&dialog));
        for (QWidget *control : std::initializer_list<QWidget *>{button, field, choice, seconds})
            QVERIFY2(control->height() == 24, qPrintable(QStringLiteral("%1 is %2").arg(control->metaObject()->className()).arg(control->height())));
        for (QAbstractButton *push : buttons->buttons()) {
            QCOMPARE(push->height(), 24);
            QVERIFY(push->width() >= 64);
        }
        // Flat: a button's fill is the same at its top and foot, and its
        // corner is clear (6pt radius).
        const QImage shot = dialog.grab().toImage();
        const qreal ratio = shot.devicePixelRatio();
        const QRect bezel = button->geometry();
        const auto at = [&](int x, int y) { return shot.pixelColor(QPoint(x, y) * ratio).rgb(); };
        QCOMPARE(at(bezel.center().x(), bezel.top() + 3), editorTheme().button.rgb());
        QCOMPARE(at(bezel.center().x(), bezel.bottom() - 3), editorTheme().button.rgb());
        QCOMPARE(at(bezel.left(), bezel.top()), editorTheme().dialog.rgb());
        // The default button is the flat accent.
        QPushButton *ok = buttons->button(QDialogButtonBox::Ok);
        QVERIFY(ok->isDefault());
        const QRect okBezel(ok->mapTo(&dialog, QPoint()), ok->size());
        QCOMPARE(at(okBezel.center().x(), okBezel.top() + 3), editorTheme().accent.rgb());
        QCOMPARE(at(okBezel.center().x(), okBezel.bottom() - 3), editorTheme().accent.rgb());
        // A field is `raised`.
        QCOMPARE(at(field->geometry().left() + 4, field->geometry().center().y()), editorTheme().field.rgb());
        // Check boxes and the slider's knob.
        QCOMPARE(QApplication::style()->pixelMetric(QStyle::PM_IndicatorWidth), 14);
        QCOMPARE(QApplication::style()->pixelMetric(QStyle::PM_SliderLength), 10);
        QCOMPARE(QApplication::style()->pixelMetric(QStyle::PM_ScrollBarExtent), 0);
        // The list scrolls, its bar hidden: the bar takes no width.
        QVERIFY(list->verticalScrollBar()->maximum() > 0);
        QCOMPARE(list->verticalScrollBar()->isVisible() ? list->verticalScrollBar()->width() : 0, 0);
        // Menus: rows 28pt.
        QMenu menu;
        menu.addAction(QStringLiteral("Render preview"));
        menu.addAction(QStringLiteral("Open Deliver workspace"));
        menu.popup(dialog.mapToGlobal(QPoint(10, 10)));
        QVERIFY(QTest::qWaitForWindowExposed(&menu));
        for (QAction *action : menu.actions()) QCOMPARE(menu.actionGeometry(action).height(), 28);
        QVERIFY(menu.testAttribute(Qt::WA_TranslucentBackground));
        menu.close();
        // The tooltip is a rounded card: a 1pt line2 edge, 3pt above and
        // below its one 12px line and 8pt either side (24pt with the UI
        // face's 16pt line).
        QToolTip::showText(dialog.mapToGlobal(QPoint(20, 20)), QStringLiteral("Export master"), &dialog);
        QWidget *tip = nullptr;
        QTRY_VERIFY((tip = [] {
            for (QWidget *widget : QApplication::topLevelWidgets())
                if (widget->inherits("QTipLabel") && widget->isVisible()) return widget;
            return static_cast<QWidget *>(nullptr);
        }()));
        QCOMPARE(tip->contentsMargins(), QMargins(9, 4, 9, 4));
        QCOMPARE(tip->font().pixelSize(), 12);
        QVERIFY(tip->testAttribute(Qt::WA_TranslucentBackground));
        QToolTip::hideText();
        // Tooltips wake after 400ms and come back at once within 300ms.
        QCOMPARE(QApplication::style()->styleHint(QStyle::SH_ToolTip_WakeUpDelay), 400);
        QCOMPARE(QApplication::style()->styleHint(QStyle::SH_ToolTip_FallAsleepDelay), 300);
    }

    // The theme never changes now, so EditorAppearance::changed fires only
    // when what cached pixmaps depend on does: the application font (or a
    // screen's density). The theme it re-applies is the same one.
    void appearanceChangesOnlyForFontsAndScreens() {
        QSignalSpy changed(EditorAppearance::instance(), &EditorAppearance::changed);
        const QColor accent = editorTheme().accent;
        applyEditorAppearance();
        applyEditorAppearance();
        QCOMPARE(changed.count(), 0);
        const QFont was = QApplication::font();
        const auto restore = qScopeGuard([was] { QApplication::setFont(was); });
        QFont bigger = was;
        bigger.setPixelSize(was.pixelSize() > 0 ? was.pixelSize() + 1 : 14);
        QApplication::setFont(bigger);
        QTRY_COMPARE(changed.count(), 1);
        QCOMPARE(editorTheme().accent, accent);
        QCOMPARE(QGuiApplication::palette().color(QPalette::Active, QPalette::Base), editorTheme().field);
        QVERIFY(qApp->styleSheet().contains(editorTheme().chrome.name()));
    }

    // The viewers, the Source monitor's scrub bar, bin tiles and effect
    // cards draw in the theme.
    void paintedPanelsFollowTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        // Footage (the harbor at dawn: sky, hills and water), on which the
        // masks' outlines and names must read.
        const QImage picture = testing::footageFrame(testing::Shot::HarborWide, QSize(960, 540));
        MediaArtwork artwork;
        artwork.frames.insert(QStringLiteral("m1"), picture.scaled(160, 90));
        artwork.frames.insert(QStringLiteral("m2"), picture.scaled(160, 90));
        ProjectPanel bin(&artwork);
        bin.setSources(QJsonArray{
            QJsonObject{{"id", "m1"}, {"path", "media/a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m2"}, {"path", "media/b.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m3"}, {"path", "media/c.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}},
            QJsonObject{{"id", "m4"}, {"path", "media/d.wav"}, {"duration", 4}}});
        bin.setMediaStatus(QJsonObject{{"missing", QJsonArray{"m3"}}, {"pointers", QJsonArray{"m2"}}, {"external", QJsonArray{"m1"}}});
        bin.resize(560, 150);
        const auto tilePixel = [&bin](int row, const QPoint &at) {
            return bin.m_list->item(row)->icon().pixmap(ProjectPanel::kThumbnail).toImage().pixelColor(at);
        };
        {
            const EditorTheme &theme = editorTheme();
            const QString name = QStringLiteral("dark");

            // The picture region: a picture, a caption, and two masks, one selected.
            ViewerWidget viewer;
            viewer.resize(640, 380);
            viewer.setProjectInfo(1920, 1080);
            viewer.setFrame(picture, 0.0);
            viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
            viewer.setMasks(QStringLiteral("c1"), QJsonArray{
                QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.2, 0.15, 0.55, 0.6}}, {"mode", "add"}},
                QJsonObject{{"name", "k2"}, {"ellipse", QJsonArray{0.75, 0.4, 0.12, 0.2}}, {"mode", "none"}}},
                QTransform::fromScale(1920, 1080));
            viewer.selectMask(QStringLiteral("k1"));
            const QImage view = viewer.grab().toImage();
            const qreal ratio = view.devicePixelRatio();
            // Black, with at most a little of the picture's light.
            QVERIFY(view.pixelColor(QPoint(2, 2) * ratio).lightness() < 16);
            const QPointF corner = viewer.pictureToWidget().map(QPointF(0.2, 0.15));
            QCOMPARE(view.pixelColor((corner * ratio).toPoint()), theme.maskHandleFill);
            // The selected mask's name is light over the picture.
            const QRect nameArea = QRectF(corner + QPointF(3, -16), QSizeF(16, 13)).toAlignedRect();
            int lightest = 0;
            for (int y = nameArea.top(); y <= nameArea.bottom(); ++y)
                for (int x = nameArea.left(); x <= nameArea.right(); ++x)
                    lightest = std::max(lightest, view.pixelColor(QPoint(x, y) * ratio).lightness());
            QVERIFY2(lightest > 240, qPrintable(QString::number(lightest)));
            QCOMPARE(amberPixels(view), 0);
            save(view, QStringLiteral("viewer-%1").arg(name));
            viewer.clearFrame();
            viewer.setMasks(QString(), {}, {});
            viewer.setCaption(QString());
            const QImage empty = viewer.grab().toImage();
            // Empty, the picture is the canvas.
            QCOMPARE(empty.pixelColor((viewer.videoRect().topLeft() + QPointF(6, 6)).toPoint() * ratio), theme.viewerCanvas);
            save(empty, QStringLiteral("viewer-empty-%1").arg(name));

            // The focus region picker.
            RegionPicker picker(picture.scaled(480, 270), QRectF(0.3, 0.2, 0.4, 0.5));
            picker.resize(520, 290);
            const QImage region = picker.grab().toImage();
            QCOMPARE(region.pixelColor(QPoint(2, 2) * region.devicePixelRatio()), theme.viewerSurround);
            QCOMPARE(amberPixels(region), 0);
            save(region, QStringLiteral("region-%1").arg(name));

            // Bin thumbs (72 x 38, mockup-spec 6): sound without peaks on
            // raised, offline media black saying so, a status as a dot.
            const auto tileAt = [&bin](int row, const QPoint &at) {
                const QImage tile = bin.m_list->item(row)->icon().pixmap(ProjectPanel::kThumbnail).toImage();
                return tile.pixelColor(at * tile.devicePixelRatio()).rgb();
            };
            QCOMPARE(tileAt(3, QPoint(2, 2)), tokens::raised);
            QCOMPARE(tileAt(2, QPoint(2, 2)), qRgb(0, 0, 0));
            QVERIFY(nearColor(tileAt(1, QPoint(7, 7)), tokens::accentInk, 8));
            QVERIFY(nearColor(tileAt(0, QPoint(7, 7)), tokens::ink3, 8));
            bin.show();
            QVERIFY(QTest::qWaitForWindowExposed(&bin));
            const QImage tiles = bin.grab().toImage();
            QCOMPARE(amberPixels(tiles), 0);
            save(tiles, QStringLiteral("bin-%1").arg(name));

        }
    }

    // The whole editor, a portrait project with a clip selected, to look at:
    // run with KERF_EDITOR_GALLERY set to a folder (QT_SCALE_FACTOR=2 for a
    // Retina screen's pixels).
    void editorGallery() {
        const QString folder = qEnvironmentVariable("KERF_EDITOR_GALLERY");
        if (folder.isEmpty()) QSKIP("Set KERF_EDITOR_GALLERY to draw the editor.");
        QDir().mkpath(folder);
        MainWindow window{QString(), QString()};
        window.resize(1366, 737);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
        QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
        timeline.insert(QStringLiteral("width"), 1080);
        timeline.insert(QStringLiteral("height"), 1920);
        snapshot.insert(QStringLiteral("timeline"), timeline);
        window.applySnapshot(snapshot);
        window.selectClip(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QTest::qWait(50);
        QVERIFY(window.grab().save(QDir(folder).filePath(QStringLiteral("editor-dark.png"))));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ThemeTest)

#include "theme_test.moc"
