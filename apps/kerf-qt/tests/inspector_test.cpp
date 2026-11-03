// Tests for the Inspector and its controls.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "device_frame.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "player_overlay.h"
#include "presets_panel.h"
#include "program_level.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "workspace.h"

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
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStatusBar>
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
#include <QWheelEvent>
#include <QWidget>

#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockAreaWidget.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>
#include <ElidingLabel.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

namespace {

// Whether `image` has any paint outside the device pixels `logical` covers.
bool paintsOutside(const QImage &image, const QRectF &logical) {
    const qreal ratio = image.devicePixelRatio();
    const QRect inside(QPoint(qFloor(logical.left() * ratio), qFloor(logical.top() * ratio)),
                       QPoint(qCeil(logical.right() * ratio) - 1, qCeil(logical.bottom() * ratio) - 1));
    for (int y = 0; y < image.height(); ++y)
        for (int x = 0; x < image.width(); ++x)
            if (!inside.contains(x, y) && qAlpha(image.pixel(x, y)) != 0) return true;
    return false;
}

} // namespace

class InspectorTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // A slider never jumps on a stray click: a press away from its knob
    // changes nothing until the pointer moves, then moves the knob by as
    // much; a press on the knob takes it.
    void inspectorSliderIgnoresStrayClicks() {
        InspectorSlider slider;
        slider.resize(92, ScrubField::kHeight);
        slider.setRange(0.0, 100.0);
        slider.setOrigin(0.0);
        slider.setDetent(0.0);
        slider.setValue(50.0);
        QSignalSpy released(&slider, &InspectorSlider::released);
        const double knob = slider.knobCenter().x();
        // A click on the groove far from the knob: nothing.
        mouse(&slider, QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(10, 12));
        QCOMPARE(slider.value(), 50.0);
        QCOMPARE(released.size(), 0);
        // A drag from there: the knob moves by the pointer's travel.
        mouse(&slider, QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(&slider, QEvent::MouseMove, QPointF(20, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(20, 12));
        QCOMPARE(released.size(), 1);
        QVERIFY(std::abs(slider.knobCenter().x() - (knob + 10.0)) < 0.5);
        QCOMPARE(released.constLast().at(1).toDouble(), 50.0);
        // A press on the knob off its centre: the first move does not jump
        // the value by the offset; the knob moves by the pointer's travel.
        const double at = slider.knobCenter().x();
        const double grabbed = slider.value();
        mouse(&slider, QEvent::MouseButtonPress, QPointF(at + 5, 12));
        mouse(&slider, QEvent::MouseMove, QPointF(at + 4, 12));
        QVERIFY2(std::abs(slider.knobCenter().x() - (at - 1.0)) < 0.5, qPrintable(QString::number(slider.knobCenter().x() - at)));
        QVERIFY(slider.value() < grabbed);
        mouse(&slider, QEvent::MouseMove, QPointF(at - 20, 12));
        mouse(&slider, QEvent::MouseButtonRelease, QPointF(at - 20, 12));
        QVERIFY(std::abs(slider.knobCenter().x() - (at - 25.0)) < 0.5);
        QCOMPARE(released.size(), 2);
        // The fill runs from the origin to the knob's centre, never past it;
        // at the origin there is none.
        InspectorSlider wide;
        wide.resize(84, ScrubField::kHeight);
        wide.setRange(-3.0, 3.0);
        wide.setOrigin(0.0);
        wide.setDetent(0.0);
        wide.setValue(0.0);
        QVERIFY(wide.fillRect().isEmpty());
        for (const double value : {0.1, 0.4, -0.2, 2.0}) {
            wide.setValue(value);
            const QRectF fill = wide.fillRect();
            const double knob = wide.knobCenter().x();
            if (value > 0) QVERIFY2(std::abs(fill.right() - knob) < 0.5, qPrintable(QString::number(value)));
            else QVERIFY2(std::abs(fill.left() - knob) < 0.5, qPrintable(QString::number(value)));
        }
    }

    // The default catches the knob on one pixel only: dragged slowly past
    // it on the Inspector's 84-point slider, every pixel gives a value of
    // its own, one step from the last, and exactly one gives the default.
    // ⌥ moves it a tenth as far; a double-click on the knob resets it.
    void inspectorSliderReachesValuesNearTheDefault() {
        const auto drag = [](InspectorSlider &slider, QEvent::Type type, double x, Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
            const QPointF at(x, ScrubField::kHeight / 2.0);
            const Qt::MouseButton button = type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton;
            const Qt::MouseButtons buttons = type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::MouseButtons(Qt::LeftButton);
            QMouseEvent event(type, at, slider.mapToGlobal(at), button, buttons, modifiers);
            QCoreApplication::sendEvent(&slider, &event);
        };
        struct Case {
            const char *what;
            double minimum, maximum, detent;
            bool gain;
        };
        // Saturation (0-400%, default 100%), Exposure (±3), Volume (0 dB).
        for (const Case &c : {Case{"saturation", 0.0, 4.0, 1.0, false}, Case{"exposure", -3.0, 3.0, 0.0, false},
                              Case{"volume", -96.0, 24.0, 0.0, true}}) {
            InspectorSlider slider;
            slider.resize(84, ScrubField::kHeight);
            slider.setRange(c.minimum, c.maximum);
            slider.setGainTravel(c.gain);
            slider.setOrigin(c.detent);
            slider.setDetent(c.detent);
            slider.setValue(c.detent);
            const double knob = slider.knobCenter().x();
            const int from = int(std::floor(knob)) - 8;
            const int to = int(std::floor(knob)) + 8;
            drag(slider, QEvent::MouseButtonPress, std::floor(knob));
            QList<double> values;
            for (int x = from; x <= to; ++x) {
                drag(slider, QEvent::MouseMove, x);
                values << slider.value();
            }
            drag(slider, QEvent::MouseButtonRelease, to);
            int atDefault = 0;
            for (int i = 0; i < values.size(); ++i) {
                if (values.at(i) == c.detent) ++atDefault;
                if (i == 0) continue;
                // Each pixel its own value, in order: none hidden behind the default.
                QVERIFY2(values.at(i) > values.at(i - 1), qPrintable(QStringLiteral("%1 at %2: %3 after %4").arg(c.what).arg(from + i)
                                                                        .arg(values.at(i)).arg(values.at(i - 1))));
                // No jump: the travel between two pixels is at most a pixel
                // and a half's (the caught pixel is at most half a pixel off).
                const double step = slider.travelOf(values.at(i)) - slider.travelOf(values.at(i - 1));
                const double pixel = 1.0 / slider.groove().width();
                QVERIFY2(step <= 1.5 * pixel + 1e-9, qPrintable(QStringLiteral("%1 at %2: %3 px").arg(c.what).arg(from + i).arg(step / pixel)));
            }
            QVERIFY2(atDefault == 1, qPrintable(QStringLiteral("%1: %2 pixels give the default").arg(c.what).arg(atDefault)));
        }
        InspectorSlider slider;
        slider.resize(84, ScrubField::kHeight);
        slider.setRange(0.0, 4.0);
        slider.setOrigin(0.0);
        slider.setDetent(1.0);
        slider.setValue(1.0);
        QSignalSpy released(&slider, &InspectorSlider::released);
        const double knob = slider.knobCenter().x();
        const double pixel = 4.0 / slider.groove().width();
        // 3 points left of the default: about 0.83, not the default.
        drag(slider, QEvent::MouseButtonPress, knob);
        drag(slider, QEvent::MouseMove, knob - 3.0);
        drag(slider, QEvent::MouseButtonRelease, knob - 3.0);
        QVERIFY2(std::abs(slider.value() - (1.0 - 3.0 * pixel)) < 1e-9, qPrintable(QString::number(slider.value())));
        QCOMPARE(released.size(), 1);
        // With ⌥ held, 20 points of travel move it 2.
        slider.setValue(1.0);
        drag(slider, QEvent::MouseButtonPress, knob, Qt::AltModifier);
        drag(slider, QEvent::MouseMove, knob - 20.0, Qt::AltModifier);
        drag(slider, QEvent::MouseButtonRelease, knob - 20.0, Qt::AltModifier);
        QVERIFY2(std::abs(slider.value() - (1.0 - 2.0 * pixel)) < 1e-9, qPrintable(QString::number(slider.value())));
        // A double-click on the knob: back to the default, one edit.
        const int before = int(released.size());
        const double changed = slider.value();
        drag(slider, QEvent::MouseButtonPress, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonRelease, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonDblClick, slider.knobCenter().x());
        drag(slider, QEvent::MouseButtonRelease, slider.knobCenter().x());
        QCOMPARE(slider.value(), 1.0);
        QCOMPARE(int(released.size()), before + 1);
        QCOMPARE(released.constLast().at(0).toDouble(), 1.0);
        QCOMPARE(released.constLast().at(1).toDouble(), changed);
    }

    // A slider's default stays in sight in both appearances without
    // anything drawn on the knob: under the knob the 1pt tick's ends show
    // above and below it, crisp, in the detent ink; the knob's inside is
    // its own colour only. A changed value's knob edge stands out against
    // the groove and panel; at the default it is quieter.
    void inspectorSliderDefaultShowsAroundTheKnob() {
        {
            applyEditorAppearance();
            const EditorTheme &theme = editorTheme();
            // The knob's edge is read against what lies round it: a changed
            // knob's is the more visible one against both the groove and
            // the panel, at least 3:1 on the groove, in both appearances.
            const QString look = QStringLiteral("dark");
            QVERIFY(theme.sliderHandleChanged != theme.sliderHandleAtDefault);
            for (const QColor &around : {theme.sliderGroove, theme.panel})
                QVERIFY2(contrastRatio(theme.sliderHandleChanged, around) > contrastRatio(theme.sliderHandleAtDefault, around),
                         qPrintable(look + QStringLiteral(" on ") + around.name()));
            QVERIFY2(contrastRatio(theme.sliderHandleChanged, theme.sliderGroove) >= 3.0, qPrintable(look));
            QVERIFY2(contrastRatio(theme.sliderHandleChanged, theme.panel) >= 3.0, qPrintable(look));
            InspectorSlider slider;
            slider.resize(92, ScrubField::kHeight);
            slider.setRange(-3.0, 3.0);
            slider.setOrigin(0.0);
            slider.setDetent(0.0);
            for (const double value : {0.0, 0.1}) {
                slider.setValue(value);
                const QString what = QStringLiteral("%1 %2").arg("dark").arg(value);
                QCOMPARE(slider.knobEdge(), value == 0.0 ? theme.sliderHandleAtDefault : theme.sliderHandleChanged);
                const QRect tick = slider.detentTick();
                const QPointF knob = slider.knobCenter();
                QCOMPARE(tick.width(), 1);
                QVERIFY2(tick.top() <= knob.y() - InspectorSlider::kKnobRadius - 3.0, qPrintable(what));
                QVERIFY2(tick.bottom() + 1 >= knob.y() + InspectorSlider::kKnobRadius + 3.0, qPrintable(what));
                QImage image(slider.size(), QImage::Format_ARGB32_Premultiplied);
                image.fill(theme.panel);
                slider.render(&image);
                // The tick's ends: the detent ink, solid, above and below.
                for (const int y : {tick.top(), tick.top() + 2, tick.bottom() - 2, tick.bottom()})
                    QVERIFY2(image.pixelColor(tick.left(), y) == theme.inspectorDetent, qPrintable(what + QStringLiteral(" y %1").arg(y)));
                // Inside the knob (clear of its 1pt edge): the knob's colour only.
                for (int y = 0; y < image.height(); ++y)
                    for (int x = 0; x < image.width(); ++x) {
                        const QPointF centre(x + 0.5, y + 0.5);
                        if (QLineF(centre, knob).length() > InspectorSlider::kKnobRadius - 1.5) continue;
                        QVERIFY2(image.pixelColor(x, y) == theme.sliderHandle,
                                 qPrintable(what + QStringLiteral(" at %1,%2").arg(x).arg(y)));
                    }
            }
            // Away from the knob, the plain 1x10 mark.
            slider.setValue(2.0);
            QCOMPARE(slider.detentTick().height(), 10);
        }
    }

    // A switched-off effect's values stay editable and readable: numbers,
    // slider fills and the colour's hex in the secondary ink (4.5:1 on the
    // field), never the disabled one.
    void switchedOffEffectValuesStayReadable() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}, {"effects", QJsonArray{
                                    QJsonObject{{"fx", "light.edgeglow"}, {"enabled", false}, {"params", QJsonObject{{"radius", 40}, {"intensity", 200}}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        QWidget *box = inspector->findChild<QWidget *>(QStringLiteral("effectBox"));
        QVERIFY(box);
        QVERIFY(!box->findChild<PowerKey *>(QStringLiteral("effectEnabled"))->isChecked());
        const EditorTheme &theme = editorTheme();
        QVERIFY(contrastRatio(theme.textSecondary, theme.inspectorField) >= 4.5);
        const auto fields = box->findChildren<ScrubField *>();
        QVERIFY(!fields.isEmpty());
        for (ScrubField *field : fields) {
            QVERIFY(field->isDimmed());
            QCOMPARE(field->valueInk(), theme.textSecondary);
        }
        const auto sliders = box->findChildren<InspectorSlider *>();
        QVERIFY(!sliders.isEmpty());
        for (InspectorSlider *slider : sliders) {
            QVERIFY(slider->isDimmed() && slider->isEnabled());
            QCOMPARE(slider->fillInk(), theme.textSecondary);
        }
        for (QLabel *hex : box->findChildren<QLabel *>(QStringLiteral("colorHex"))) {
            QVERIFY(hex->property("off").toBool());
            QCOMPARE(hex->palette().color(QPalette::WindowText), theme.textSecondary);
        }
    }

    // Each effect is a header with a switch, its name and ⋯ (Reset, Ask
    // Kerf, Remove), no ✕; folded, a second line lists what changed; its
    // parameters are scrub fields with units, five before "N more", grouped
    // under quiet sub-headers when the catalog groups them.
    void inspectorShowsAndEditsTheEffectStack() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0},
                                {"duration", 2.0}, {"effects", QJsonArray{
                                    QJsonObject{{"fx", "light.edgeglow"}, {"enabled", true}, {"params", QJsonObject{{"radius", 40}, {"intensity", 200}}}},
                                    QJsonObject{{"fx", "channel.invert"}, {"enabled", false}, {"params", QJsonObject{}}}}}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        InspectorSection *effects = inspector->section(QStringLiteral("effects"));
        QVERIFY(effects);
        // Folded, its header names every effect it counts, one switched
        // off marked so.
        QVERIFY(!effects->isOpen());
        QCOMPARE(effects->summary(), QStringLiteral("Edge Glow, Invert (off)"));
        // Shown in whole names only: the ones on, then the count switched
        // off; short of room, "1 on · 1 off"; shorter still, nothing.
        {
            QFont font;
            font.setPixelSize(12);
            const QFontMetrics metrics(font);
            InspectorSection::Summary summary{{QStringLiteral("Primary Colour Correction"), QStringLiteral("Gaussian Blur")},
                                              QStringLiteral(" · 1 off"), QStringLiteral("2 on · 1 off"), QString()};
            const auto fit = [&](const QString &text) { return InspectorSection::fitSummary(summary, metrics, metrics.horizontalAdvance(text)); };
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction, Gaussian Blur · 1 off")), QStringLiteral("Primary Colour Correction, Gaussian Blur · 1 off"));
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction +1 · 1 off")), QStringLiteral("Primary Colour Correction +1 · 1 off"));
            QCOMPARE(fit(QStringLiteral("Primary Colour Correction")), QStringLiteral("2 on · 1 off"));
            QCOMPARE(InspectorSection::fitSummary(summary, metrics, 8), QString());
            QCOMPARE(effects->shownSummary().contains(QChar(0x2026)), false);
        }
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 2);
        auto *glow = boxes.at(0)->findChild<PowerKey *>(QStringLiteral("effectEnabled"));
        auto *radius = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("effectParam_radius"));
        auto *intensity = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("effectParam_intensity"));
        QVERIFY(glow && radius && intensity);
        QCOMPARE(boxes.at(0)->findChild<QLabel *>(QStringLiteral("effectName"))->text(), QStringLiteral("Edge Glow"));
        QVERIFY(glow->isChecked());
        QCOMPARE(radius->value(), 40.0);
        // "Intensity 150.000" reads "200%" now, with a slider.
        QCOMPARE(intensity->text(), QStringLiteral("200"));
        QCOMPARE(intensity->format().unit, QStringLiteral("%"));
        QVERIFY(boxes.at(0)->findChild<InspectorSlider *>(QStringLiteral("slider_intensity")));
        QVERIFY(!boxes.at(1)->findChild<PowerKey *>(QStringLiteral("effectEnabled"))->isChecked());
        QVERIFY(boxes.at(1)->findChild<QLabel *>(QStringLiteral("effectName"))->property("off").toBool());
        // Folded, the second line lists the values that differ.
        auto *summary = boxes.at(0)->findChild<QLabel *>(QStringLiteral("effectSummary"));
        QCOMPARE(summary->text(), QStringLiteral("Glow radius 40 px · Intensity 200%"));
        QVERIFY(!summary->isHidden());
        QVERIFY(boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        // No ✕ anywhere.
        for (QAbstractButton *button : inspector->findChildren<QAbstractButton *>()) {
            QVERIFY(button->text() != QStringLiteral("✕"));
            QVERIFY(button->objectName() != QStringLiteral("effectRemove"));
        }
        // Opening it shows its rows and hides the line; it stays open for the effect.
        boxes.at(0)->findChild<GlyphButton *>(QStringLiteral("effectChevron"))->click();
        QVERIFY(!boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        QVERIFY(summary->isHidden());
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/fx/light.edgeglow/open")).toBool());
        // Row labels keep one 12-point line: a first word the effect's name
        // says goes ("Edge Threshold" under Edge Glow reads "Threshold"),
        // the catalog's label whole in the tooltip; "Glow only" keeps its
        // first word, since "Only" alone says nothing.
        {
            const auto labelOf = [&](const char *param) -> RowLabel * {
                for (QWidget *at = boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParam_%1").arg(QLatin1String(param))); at; at = at->parentWidget())
                    if (auto *row = qobject_cast<InspectorRow *>(at)) return row->label();
                return nullptr;
            };
            RowLabel *threshold = labelOf("threshold");
            QVERIFY(threshold);
            QCOMPARE(threshold->text(), QStringLiteral("Threshold"));
            QCOMPARE(threshold->toolTip(), QStringLiteral("Edge Threshold"));
            QCOMPARE(labelOf("width")->text(), QStringLiteral("Width"));
            QCOMPARE(labelOf("radius")->text(), QStringLiteral("Radius"));
            QCOMPARE(labelOf("color")->text(), QStringLiteral("Colour"));
            QCOMPARE(labelOf("color")->toolTip(), QStringLiteral("Glow Colour"));
            QCOMPARE(labelOf("only")->text(), QStringLiteral("Glow only"));
            for (RowLabel *label : boxes.at(0)->findChildren<RowLabel *>()) {
                QCOMPARE(label->font().pixelSize(), 12);
                QVERIFY(!label->wordWrap() && !label->text().contains(QLatin1Char('\n')));
                QVERIFY2(label->sizeHint().height() <= QFontMetrics(label->font()).lineSpacing() + 2, qPrintable(label->text()));
            }
            // Too long for the column even so: one line, ended in an ellipsis.
            RowLabel lengthy(QStringLiteral("Highlight threshold of the brightest spots"));
            lengthy.resize(InspectorRow::kLabelWidth, 20);
            lengthy.show();
            QCoreApplication::processEvents();
            QVERIFY(lengthy.text().endsWith(QChar(0x2026)) && !lengthy.text().contains(QLatin1Char('\n')));
            QCOMPARE(lengthy.toolTip(), QStringLiteral("Highlight threshold of the brightest spots"));
            QCOMPARE(lengthy.font().pixelSize(), 12);
        }
        // Names and labels read as the rest of the UI: effects in Title
        // Case, labels in sentence case, "colour" spelt as elsewhere.
        QCOMPARE(presentationEffectName(QStringLiteral("Primary color correction.")), QStringLiteral("Primary Colour Correction"));
        QCOMPARE(presentationEffectName(QStringLiteral("Gaussian blur.")), QStringLiteral("Gaussian Blur"));
        QCOMPARE(presentationEffectName(QStringLiteral("Edge Glow")), QStringLiteral("Edge Glow"));
        QCOMPARE(sentenceCase(QStringLiteral("Look LUT")), QStringLiteral("Look LUT"));
        QCOMPARE(sentenceCase(QStringLiteral("Glow Color")), QStringLiteral("Glow colour"));
        // Edge Glow has six parameters: folding one would save nothing, so
        // all six show and there is no "1 more".
        {
            QVERIFY(!boxes.at(0)->findChild<QPushButton *>(QStringLiteral("effectMoreToggle")));
            QWidget *moreRows = boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectMoreRows"));
            int params = 0;
            for (QWidget *child : boxes.at(0)->findChildren<QWidget *>())
                if (child->objectName().startsWith(QStringLiteral("effectParam_"))) {
                    ++params;
                    QVERIFY2(!moreRows || !moreRows->isAncestorOf(child), qPrintable(child->objectName()));
                }
            QCOMPARE(params, 6);
        }
        auto op = [&window] { return takeOps(window).first().toObject(); };
        glow->click();
        QJsonObject edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(edit.value("index").toInt(), 0);
        QCOMPARE(edit.value("enabled").toBool(), false);
        radius->commitText(QStringLiteral("10")); // the default
        edit = op();
        QVERIFY(edit.value("params").toObject().contains("radius"));
        QVERIFY(edit.value("params").toObject().value("radius").isNull());
        // ⋯ ▸ Remove and ⋯ ▸ Reset Parameters.
        boxes.at(1)->findChild<QAction *>(QStringLiteral("effectRemove"))->trigger();
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-remove"));
        QCOMPARE(edit.value("index").toInt(), 1);
        boxes.at(0)->findChild<QAction *>(QStringLiteral("effectReset"))->trigger();
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-set"));
        QVERIFY(edit.value("params").toObject().value("radius").isNull());
        QVERIFY(edit.value("params").toObject().value("intensity").isNull());
        // Delete on a focused effect header removes it.
        key(boxes.at(1)->findChild<QWidget *>(QStringLiteral("effectHeader")), Qt::Key_Delete);
        edit = op();
        QCOMPARE(edit.value("op").toString(), QStringLiteral("fx-remove"));
        QCOMPARE(edit.value("index").toInt(), 1);
        // ⋯ ▸ Ask Kerf about This Effect.
        QSignalSpy asked(inspector, &InspectorPanel::askRequested);
        boxes.at(0)->findChild<QAction *>(QStringLiteral("effectAsk"))->trigger();
        QCOMPARE(asked.size(), 1);
        QVERIFY(asked.first().first().toString().endsWith(QStringLiteral("Edge Glow")));

        // Grouped parameters ("Vignette: Amount") sit under sub-headers, and
        // "N more" counts the groups after the first.
        inspector->m_effectDefs.insert(QStringLiteral("test.grouped"), QJsonObject{{"id", "test.grouped"}, {"name", "Grouped"}, {"params", QJsonArray{
            QJsonObject{{"name", "temperature"}, {"type", "number"}, {"default", 0}, {"min", -100}, {"max", 100}, {"doc", "Basic Correction: Temperature"}},
            QJsonObject{{"name", "tint"}, {"type", "number"}, {"default", 0}, {"min", -100}, {"max", 100}, {"doc", "Basic Correction: Tint"}},
            QJsonObject{{"name", "amount"}, {"type", "number"}, {"default", 0}, {"min", -5}, {"max", 5}, {"doc", "Vignette: Amount"}},
            QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 50}, {"min", 0}, {"max", 100}, {"doc", "Vignette: Feather"}},
            QJsonObject{{"name", "grain"}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 100}, {"doc", "Film: Grain"}}}}});
        inspector->m_shape.clear();
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                            {"effects", QJsonArray{QJsonObject{{"fx", "test.grouped"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        QWidget *grouped = inspector->findChildren<QWidget *>(QStringLiteral("effectBox")).value(0);
        QVERIFY(grouped);
        QStringList groups;
        for (QLabel *label : grouped->findChildren<QLabel *>(QStringLiteral("effectGroup"))) groups << label->text();
        QCOMPARE(groups, (QStringList{"Basic Correction", "Vignette", "Film"}));
        QCOMPARE(grouped->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"))->text(), QStringLiteral("2 more"));
        QWidget *amount = grouped->findChild<ScrubField *>(QStringLiteral("effectParam_amount"));
        QVERIFY(amount && grouped->findChild<QWidget *>(QStringLiteral("effectMoreRows"))->isAncestorOf(amount));
        auto *tintRow = qobject_cast<InspectorRow *>(grouped->findChild<ScrubField *>(QStringLiteral("effectParam_tint"))->parentWidget());
        QVERIFY(tintRow);
        QCOMPARE(tintRow->label()->toolTip(), QStringLiteral("Basic Correction: Tint"));

        // Seven parameters: five, then "2 more ⌄", in words as Add effect
        // is, at the column's start; open stays open per effect.
        QJsonArray seven;
        for (int index = 0; index < 7; ++index)
            seven.append(QJsonObject{{"name", QStringLiteral("p%1").arg(index)}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 100},
                                     {"doc", QStringLiteral("Amount %1").arg(index)}});
        inspector->m_effectDefs.insert(QStringLiteral("test.seven"), QJsonObject{{"id", "test.seven"}, {"name", "Seven"}, {"params", seven}});
        inspector->m_shape.clear();
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                            {"effects", QJsonArray{QJsonObject{{"fx", "test.seven"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        QWidget *long7 = inspector->findChildren<QWidget *>(QStringLiteral("effectBox")).value(0);
        QVERIFY(long7);
        auto *more = long7->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"));
        QVERIFY(more);
        QCOMPARE(more->text(), QStringLiteral("2 more"));
        QCOMPARE(more->font().pixelSize(), 12);
        QCOMPARE(more->mapTo(long7, QPoint()).x(), 0);
        QWidget *moreRows = long7->findChild<QWidget *>(QStringLiteral("effectMoreRows"));
        QVERIFY(moreRows->isHidden());
        QVERIFY(moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p5"))));
        QVERIFY(!moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p4"))));
        more->click();
        QVERIFY(!moreRows->isHidden());
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/fx/test.seven/more")).toBool());
    }

    // Inspector rows for the catalog's properties of the clip's track type,
    // with units: position in px of the frame, scale and opacity in %, gain
    // in dB, pan L…R; a title's text, font, size and colour; an effect's
    // file, track and sound parameters. An animated value shows its value
    // at the playhead, read-only, with ◆ and no keyframe button.
    void inspectorEditsTheCatalogPropertiesOfTheTrackType() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_inspector->m_effectDefs.insert(QStringLiteral("test.params"), QJsonObject{{"id", "test.params"}, {"name", "Params"}, {"params", QJsonArray{
            QJsonObject{{"name", "map"}, {"type", "track"}, {"default", ""}},
            QJsonObject{{"name", "audioLayer"}, {"type", "sound"}, {"default", ""}},
            QJsonObject{{"name", "lut"}, {"type", "file"}, {"extensions", QJsonArray{"cube"}}, {"default", ""}, {"doc", "LUT"}}}}});
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()}, {"generators", catalogGenerators()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"opacity", 0.5}, {"x", QJsonArray{
                                    QJsonObject{{"at", 0.0}, {"value", 0.25}, {"interp", "linear"}},
                                    QJsonObject{{"at", 1.0}, {"value", 1.0}, {"interp", "linear"}}}}}}}}}},
                QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "g"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"props", QJsonObject{{"text", "Day One"}}}}}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", QJsonObject{{"gain", -6.0}, {"pan", -0.4}}},
                                {"effects", QJsonArray{QJsonObject{{"fx", "test.params"}, {"enabled", true}, {"params", QJsonObject{}}}}}}}}}}}}}});
        auto set = [&window] {
            const QJsonArray ops = takeOps(window);
            const QJsonObject op = ops.first().toObject();
            return op.value("op").toString() == QStringLiteral("set") ? op : QJsonObject{};
        };
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->findChildren<ScrubField *>(QStringLiteral("prop_opacity")).isEmpty());
        window.selectClip(QStringLiteral("v"));
        const auto field = [inspector](const char *name) { return inspector->findChild<ScrubField *>(QString::fromLatin1(name)); };
        ScrubField *opacity = field("prop_opacity");
        ScrubField *x = field("prop_x");
        ScrubField *scale = field("prop_scale");
        ScrubField *rotation = field("prop_rotation");
        auto *blend = inspector->findChild<QComboBox *>(QStringLiteral("prop_blend"));
        QVERIFY(opacity && x && scale && rotation && blend);
        // Units: px of the 1080-wide frame, %, °.
        QCOMPARE(opacity->text(), QStringLiteral("50"));
        QCOMPARE(opacity->format().unit, QStringLiteral("%"));
        QCOMPARE(scale->format().unit, QStringLiteral("%"));
        QCOMPARE(scale->text(), QStringLiteral("100"));
        QCOMPARE(rotation->format().unit, QStringLiteral("°"));
        QCOMPARE(x->format().unit, QStringLiteral("px"));
        QCOMPARE(x->format().prefix, QStringLiteral("X"));
        // Animated: its value at the playhead (0.25 of 1080), read-only, ◆.
        auto *xRow = qobject_cast<InspectorRow *>(x->parentWidget());
        QVERIFY(xRow);
        QVERIFY(x->isReadOnly());
        QCOMPARE(x->text(), QStringLiteral("270"));
        QVERIFY(xRow->isAnimated());
        QVERIFY(xRow->findChild<QLabel *>(QStringLiteral("animatedMark"))->toolTip().contains(QStringLiteral("2 keyframes")));
        inspector->setPlayhead(0.5);
        QCOMPARE(x->text(), QStringLiteral("675"));
        for (QWidget *widget : inspector->findChildren<QWidget *>())
            QVERIFY(!widget->objectName().contains(QStringLiteral("keyframe"), Qt::CaseInsensitive));
        QCOMPARE(blend->currentText(), QStringLiteral("Normal"));
        opacity->commitText(QStringLiteral("50%"));
        QVERIFY(takeOps(window).isEmpty());
        opacity->commitText(QStringLiteral("75"));
        QCOMPARE(set().value("props").toObject().value("opacity").toDouble(), 0.75);
        opacity->commitText(QStringLiteral("100%"));
        QVERIFY(set().value("props").toObject().value("opacity").isNull());
        blend->setCurrentIndex(1);
        emit blend->activated(1);
        QCOMPARE(set().value("props").toObject().value("blend").toString(), QStringLiteral("add"));

        // A title: its text, font, size in px of the frame's height, colour.
        window.selectClip(QStringLiteral("g"));
        auto *text = inspector->findChild<QPlainTextEdit *>(QStringLiteral("titleText"));
        auto *color = inspector->findChild<QPushButton *>(QStringLiteral("titleColor"));
        auto *font = inspector->findChild<QComboBox *>(QStringLiteral("titleFont"));
        ScrubField *size = field("titleSize");
        QVERIFY(text && color && font && size);
        QCOMPARE(text->toPlainText(), QStringLiteral("Day One"));
        // The generic family by the family it resolves to; the model keeps "sans-serif".
        QCOMPARE(font->currentData().toString(), QStringLiteral("sans-serif"));
        QVERIFY(font->currentText() != QStringLiteral("sans-serif") && !font->currentText().isEmpty());
        QCOMPARE(size->text(), QStringLiteral("154"));
        QCOMPARE(size->format().unit, QStringLiteral("px"));
        // A colour well: the swatch, with the value beside it as hex.
        QCOMPARE(color->property("modelColor").toString(), QStringLiteral("#ffffff"));
        QVERIFY(color->text().isEmpty());
        QVERIFY(!color->icon().isNull());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("colorHex"))->text(), QStringLiteral("#FFFFFF"));
        QCOMPARE(modelColor(QStringLiteral("#ff000080")).alpha(), 0x80);
        QCOMPARE(modelColorName(QColor(255, 0, 0, 0x80)), QStringLiteral("#ff000080"));
        QCOMPARE(modelColorName(Qt::white), QStringLiteral("#ffffff"));
        // The text commits when it loses focus.
        text->setPlainText(QStringLiteral("Day Two"));
        QFocusEvent out(QEvent::FocusOut, Qt::OtherFocusReason);
        QCoreApplication::sendEvent(text, &out);
        QCOMPARE(set().value("props").toObject().value("text").toString(), QStringLiteral("Day Two"));
        const QJsonObject colorDef = catalogGenerators().value("title").toArray().at(3).toObject();
        inspector->commitValue(InspectorPanel::Target{InspectorPanel::Target::Kind::Prop, QStringLiteral("color")}, colorDef, QStringLiteral("#ffffff"));
        QVERIFY(takeOps(window).isEmpty());
        inspector->commitValue(InspectorPanel::Target{InspectorPanel::Target::Kind::Prop, QStringLiteral("color")}, colorDef, QStringLiteral("#ff0000"));
        QCOMPARE(set().value("props").toObject().value("color").toString(), QStringLiteral("#ff0000"));

        // A sound clip: gain in dB (−∞ at the floor), pan from L to R.
        window.selectClip(QStringLiteral("a"));
        ScrubField *gain = field("prop_gain");
        ScrubField *pan = field("prop_pan");
        QVERIFY(gain && pan);
        QCOMPARE(gain->format().unit, QStringLiteral("dB"));
        QCOMPARE(gain->text(), QString(QChar(0x2212)) + QStringLiteral("6.0"));
        QCOMPARE(pan->text(), QStringLiteral("L 40"));
        QVERIFY(inspector->findChild<InspectorSlider *>(QStringLiteral("slider_gain")));
        gain->commitText(QStringLiteral("-3dB"));
        QCOMPARE(set().value("props").toObject().value("gain").toDouble(), -3.0);
        gain->commitText(QStringLiteral("-inf"));
        QCOMPARE(set().value("props").toObject().value("gain").toDouble(), -96.0);
        QCOMPARE(gain->text(), QString(QChar(0x2212)) + QChar(0x221e));
        pan->commitText(QStringLiteral("Centre"));
        QVERIFY(set().value("props").toObject().value("pan").isNull());
        QCOMPARE(pan->text(), QStringLiteral("Centre"));
        // A file parameter is a field with a folder glyph inside its right
        // end that chooses the file; no push button beside it.
        inspector->findChild<QWidget *>(QStringLiteral("effectParams"))->show();
        QWidget *file = inspector->findChild<QWidget *>(QStringLiteral("effectParam_lut"));
        QVERIFY(file);
        auto *fileLine = file->findChild<QLineEdit *>();
        QVERIFY(fileLine && !file->findChild<QPushButton *>());
        auto *fileChoose = fileLine->findChild<QAction *>(QStringLiteral("chooseFile"));
        QVERIFY(fileChoose && !fileChoose->icon().isNull());
        QCOMPARE(fileChoose->toolTip(), QStringLiteral("Choose a file"));
        file->resize(240, 22);
        file->layout()->activate();
        QCOMPARE(fileLine->geometry().right(), file->width() - 1);
        // A track parameter picks a picture track or none; a sound one a
        // sound track or none.
        auto *track = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_map"));
        auto *sound = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_audioLayer"));
        QVERIFY(track && sound);
        QCOMPARE(inspector->m_pictureTracks, (QStringList{"V1", "V2"}));
        QCOMPARE(track->count(), 3);
        QCOMPARE(track->itemText(2), QStringLiteral("V2"));
        QCOMPARE(inspector->m_soundTracks, QStringList{"A1"});
        QCOMPARE(sound->count(), 2);
        track->setCurrentIndex(2);
        emit track->activated(2);
        const QJsonObject picked = takeOps(window).first().toObject();
        QCOMPARE(picked.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(picked.value("params").toObject().value("map").toString(), QStringLiteral("V2"));
    }

    static void keyWith(QWidget *widget, int code, Qt::KeyboardModifiers modifiers) {
        QKeyEvent event(QEvent::KeyPress, code, modifiers);
        QCoreApplication::sendEvent(widget, &event);
    }

    // Editing in the Inspector never strands the transport: a press or a
    // drag on a field takes no keyboard focus, and Return or Esc after
    // typing (or Esc with nothing typed) hands the keys back to the
    // timeline, so Space plays at once.
    void inspectorEditsLeaveTheTransportKeys() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        window.m_errorBanner->hide();
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QTRY_VERIFY(window.m_timeline->hasFocus());
        ScrubField *x = window.m_inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        ScrubField *opacity = window.m_inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QVERIFY(x && opacity && x->isVisible() && opacity->isVisible());
        QCOMPARE(x->focusPolicy(), Qt::TabFocus);
        const auto space = [&window] {
            window.m_lastCommand = {};
            QTest::keyClick(QApplication::focusWidget(), Qt::Key_Space);
            return window.m_lastCommand.value(QStringLiteral("type")).toString();
        };
        // Drag-scrub Position X by 20 points: the timeline keeps the keys.
        QTest::mousePress(x, Qt::LeftButton, {}, QPoint(30, 12));
        QTest::mouseMove(x, QPoint(40, 12));
        QTest::mouseMove(x, QPoint(50, 12));
        QTest::mouseRelease(x, Qt::LeftButton, {}, QPoint(50, 12));
        QVERIFY(window.m_timeline->hasFocus());
        QCOMPARE(space(), QStringLiteral("play"));
        // Type 50 and Return in Opacity: the edit goes, then the keys go back.
        QTest::mouseClick(opacity, Qt::LeftButton, {}, QPoint(30, 12));
        QVERIFY(opacity->isTyping() && opacity->editor()->hasFocus());
        opacity->editor()->setText(QStringLiteral("50"));
        QTest::keyClick(opacity->editor(), Qt::Key_Return);
        QVERIFY(!opacity->isTyping());
        QVERIFY(window.m_timeline->hasFocus());
        QCOMPARE(space(), QStringLiteral("play"));
        // Esc while typing reverts and hands the keys back too.
        QTest::mouseClick(opacity, Qt::LeftButton, {}, QPoint(30, 12));
        QVERIFY(opacity->isTyping());
        QTest::keyClick(opacity->editor(), Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
        // Tab reaches a field (and types in it); Esc there, then Esc again
        // anywhere in the Inspector, gives the keys back.
        x->setFocus(Qt::TabFocusReason);
        QVERIFY(x->isTyping());
        QTest::keyClick(x->editor(), Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
        window.m_inspector->setFocus(Qt::OtherFocusReason);
        QTest::keyClick(window.m_inspector, Qt::Key_Escape);
        QVERIFY(window.m_timeline->hasFocus());
    }

    // A ScrubField scrubs by dragging its value or its row's label (one
    // step per 2 points; one edit, on release), steps with ↑↓ (⇧ ×10,
    // ⌥ ÷10), takes typed values with units, relative and clamped, and draws
    // a value at its default quieter than a changed one.
    void inspectorScrubFieldsDragStepAndType() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props) {
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", props}}}}}}}}}});
        };
        snapshot(1, {});
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        ScrubField *x = inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QVERIFY(x && opacity);
        x->resize(78, ScrubField::kHeight);
        // A drag of 20 points is 10 steps of 1 px: one set, on release.
        QSignalSpy previews(x, &ScrubField::previewed);
        mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
        mouse(x, QEvent::MouseMove, QPointF(40, 12));
        mouse(x, QEvent::MouseMove, QPointF(50, 12));
        QVERIFY(previews.size() >= 2);
        QVERIFY(takeOps(window).isEmpty());
        QCOMPARE(x->text(), QStringLiteral("10"));
        mouse(x, QEvent::MouseButtonRelease, QPointF(50, 12));
        QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.first().toObject().value("op").toString(), QStringLiteral("set"));
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 10.0 / 1080.0) < 1e-9);
        // So does its row's label.
        snapshot(2, {});
        auto *row = qobject_cast<InspectorRow *>(x->parentWidget());
        QVERIFY(row && row->label()->field() == x);
        mouse(row->label(), QEvent::MouseButtonPress, QPointF(10, 12));
        mouse(row->label(), QEvent::MouseMove, QPointF(30, 12));
        mouse(row->label(), QEvent::MouseButtonRelease, QPointF(30, 12));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 10.0 / 1080.0) < 1e-9);
        // A click without a drag types: the number, selected.
        snapshot(3, {});
        mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
        mouse(x, QEvent::MouseButtonRelease, QPointF(31, 12));
        QVERIFY(x->isTyping());
        QCOMPARE(x->editor()->selectedText(), QStringLiteral("0"));
        key(x->editor(), Qt::Key_Escape);
        QVERIFY(!x->isTyping());
        QVERIFY(takeOps(window).isEmpty());
        // ↑ steps 1 px, ⇧↑ 10, ⌥↑ 0.1, where the keys are after a click on
        // the field (its line edit, which would only move its caret): the
        // text follows each step, selected, and the run with its Return is
        // one edit.
        const auto clickOpen = [&] {
            mouse(x, QEvent::MouseButtonPress, QPointF(30, 12));
            mouse(x, QEvent::MouseButtonRelease, QPointF(30, 12));
            QVERIFY(x->isTyping());
        };
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::NoModifier);
        QVERIFY(x->isStepping());
        QCOMPARE(x->editor()->text(), QStringLiteral("1"));
        QCOMPARE(x->editor()->selectedText(), QStringLiteral("1"));
        QVERIFY(takeOps(window).isEmpty());
        key(x->editor(), Qt::Key_Return);
        QVERIFY(!x->isTyping() && !x->isStepping());
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 1.0 / 1080.0) < 1e-9);
        QVERIFY(takeOps(window).isEmpty());
        snapshot(4, QJsonObject{{"x", 1.0 / 1080.0}});
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::ShiftModifier);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 11.0 / 1080.0) < 1e-9);
        snapshot(5, QJsonObject{{"x", 11.0 / 1080.0}});
        clickOpen();
        keyWith(x->editor(), Qt::Key_Up, Qt::AltModifier);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 11.1 / 1080.0) < 1e-9);
        // ↓ from a typed value; Esc takes back a run not yet sent.
        snapshot(6, QJsonObject{{"x", 0.0}});
        clickOpen();
        x->editor()->setText(QStringLiteral("20"));
        keyWith(x->editor(), Qt::Key_Down, Qt::NoModifier);
        QCOMPARE(x->editor()->text(), QStringLiteral("19"));
        key(x->editor(), Qt::Key_Escape);
        QVERIFY(!x->isStepping());
        QCOMPARE(x->text(), QStringLiteral("0"));
        QVERIFY(takeOps(window).isEmpty());
        // Held: thirty repeats are one edit (one undo step), sent once the
        // run pauses, and a snapshot arriving mid-run does not pull the value
        // back; Return then sends nothing more.
        clickOpen();
        for (int repeat = 0; repeat < 30; ++repeat) {
            QKeyEvent press(QEvent::KeyPress, Qt::Key_Up, Qt::NoModifier, QString(), repeat > 0);
            QCoreApplication::sendEvent(x->editor(), &press);
            if (repeat == 10) x->setValue(0.0);
        }
        QCOMPARE(x->editor()->text(), QStringLiteral("30"));
        QVERIFY(takeOps(window).isEmpty());
        QTRY_VERIFY_WITH_TIMEOUT(!x->isStepping(), ScrubField::kStepRunMs * 4);
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QVERIFY(std::abs(ops.first().toObject().value("props").toObject().value("x").toDouble() - 30.0 / 1080.0) < 1e-9);
        key(x->editor(), Qt::Key_Return);
        QVERIFY(takeOps(window).isEmpty());
        // Focused without typing (Return closed the editor and the keys have
        // not gone back yet), ↑ steps the field itself as well.
        snapshot(7, QJsonObject{{"x", 30.0 / 1080.0}});
        keyWith(x, Qt::Key_Up, Qt::NoModifier);
        x->flushSteps();
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("x").toDouble() - 31.0 / 1080.0) < 1e-9);
        // Typed: 50% is 0.5; +5 is relative; out of range clamps, and the
        // field flashes.
        opacity->commitText(QStringLiteral("50%"));
        QCOMPARE(takeOps(window).first().toObject().value("props").toObject().value("opacity").toDouble(), 0.5);
        snapshot(16, QJsonObject{{"opacity", 0.5}});
        opacity->commitText(QStringLiteral("+5"));
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("opacity").toDouble() - 0.55) < 1e-9);
        snapshot(17, QJsonObject{{"opacity", 0.5}});
        opacity->commitText(QStringLiteral("250"));
        QVERIFY(takeOps(window).first().toObject().value("props").toObject().value("opacity").isNull());
        QVERIFY(opacity->m_flash->isActive());
        QCOMPARE(opacity->text(), QStringLiteral("100"));
        // Parsing and formatting, units included.
        NumberFormat percent = opacity->format();
        QCOMPARE(parseNumber(percent, QStringLiteral("1.5x"), 100.0).value(), 150.0);
        QCOMPARE(parseNumber(percent, QStringLiteral("*2"), 40.0).value(), 80.0);
        QCOMPARE(formatWithUnit(percent, 50.0), QStringLiteral("50%"));
        QCOMPARE(formatWithUnit(x->format(), -48.0), QString(QChar(0x2212)) + QStringLiteral("48 px"));
        QVERIFY(!parseNumber(percent, QStringLiteral("soon"), 1.0));
        // A value at its default is drawn in the secondary ink, a changed one
        // in the accent (4.5:1 on its well).
        snapshot(18, QJsonObject{{"x", 0.1}});
        ScrubField *y = inspector->findChild<ScrubField *>(QStringLiteral("prop_y"));
        y->resize(78, ScrubField::kHeight);
        x->resize(78, ScrubField::kHeight);
        TextRecorder quiet;
        y->render(&quiet);
        QCOMPARE(quiet.colorOf(QStringLiteral("0")).rgb(), editorTheme().textSecondary.rgb());
        TextRecorder changed;
        x->render(&changed);
        QCOMPARE(changed.colorOf(QStringLiteral("108")).rgb(), changedValueInk().rgb());
        QVERIFY(contrastRatio(changedValueInk(), editorTheme().wellFill) >= 4.5);
        QCOMPARE(changed.colorOf(QStringLiteral("px")).rgb(), editorTheme().textSecondary.rgb());
    }

    // ↺ shows on a row's hover only while its value differs from the
    // default, and sends null; so does double-clicking its label. A
    // section's ↺ resets all its rows in one edit, and its folded header
    // says what changed in it.
    void inspectorRowsResetToDefaults() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props) {
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", props}}}}}}}}}});
        };
        snapshot(1, QJsonObject{{"scale", 1.12}, {"crop_l", 0.1}, {"crop_b", 0.05}});
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        auto *scaleRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"))->parentWidget());
        auto *opacityRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"))->parentWidget());
        QVERIFY(scaleRow && opacityRow);
        const auto hover = [](QWidget *widget, bool in) {
            if (in) {
                QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), QPointF(4, 4));
                QCoreApplication::sendEvent(widget, &enter);
            } else {
                QEvent leave(QEvent::Leave);
                QCoreApplication::sendEvent(widget, &leave);
            }
        };
        QVERIFY(scaleRow->isChanged() && !opacityRow->isChanged());
        QVERIFY(!scaleRow->resetButton()->isVisibleTo(scaleRow));
        hover(scaleRow, true);
        QVERIFY(scaleRow->resetButton()->isVisibleTo(scaleRow));
        hover(opacityRow, true);
        QVERIFY(!opacityRow->resetButton()->isVisibleTo(opacityRow));
        scaleRow->resetButton()->click();
        QJsonObject op = takeOps(window).first().toObject();
        QVERIFY(op.value("props").toObject().contains("scale") && op.value("props").toObject().value("scale").isNull());
        hover(scaleRow, false);
        QVERIFY(!scaleRow->resetButton()->isVisibleTo(scaleRow));
        // Double-clicking the label resets it too.
        QMouseEvent doubleClick(QEvent::MouseButtonDblClick, QPointF(4, 4), scaleRow->label()->mapToGlobal(QPointF(4, 4)),
                                Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(scaleRow->label(), &doubleClick);
        QVERIFY(takeOps(window).first().toObject().value("props").toObject().value("scale").isNull());
        // Crop folds to what changed in it, and resets in one edit.
        InspectorSection *crop = inspector->section(QStringLiteral("crop"));
        QVERIFY(crop && !crop->isOpen());
        QCOMPARE(crop->summary(), QStringLiteral("L 192 · B 54 px"));
        hover(crop->header(), true);
        QVERIFY(crop->resetButton()->isVisibleTo(crop));
        crop->resetButton()->click();
        const QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 2);
        for (const QJsonValue &value : ops) {
            const QJsonObject props = value.toObject().value("props").toObject();
            QCOMPARE(props.size(), 1);
            QVERIFY(props.begin().value().isNull());
        }
        snapshot(2, QJsonObject{{"crop_b", 0.05}});
        QCOMPARE(crop->summary(), QStringLiteral("Bottom 54 px"));
        snapshot(3, {});
        QVERIFY(crop->summary().isEmpty());
        hover(crop->header(), true);
        QVERIFY(!crop->resetButton()->isVisibleTo(crop));
        // A section opens and closes, and stays so for this kind of clip.
        crop->setOpen(true);
        emit crop->toggled(true, Qt::NoModifier);
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/folds/video")).toMap().value(QStringLiteral("crop")).toBool());
        // ⌥-click opens or closes every section; ⌘-click opens only this one.
        InspectorSection *anchor = inspector->section(QStringLiteral("anchor"));
        crop->setOpen(false);
        emit crop->toggled(false, Qt::AltModifier);
        QVERIFY(!anchor->isOpen() && !inspector->section(QStringLiteral("source"))->isOpen());
        anchor->setOpen(true);
        emit anchor->toggled(true, Qt::AltModifier);
        QVERIFY(crop->isOpen() && inspector->section(QStringLiteral("source"))->isOpen());
        emit anchor->toggled(true, Qt::ControlModifier);
        QVERIFY(anchor->isOpen() && !crop->isOpen() && !inspector->section(QStringLiteral("source"))->isOpen());
    }

    // Each module's fold is kept by id, so one added since (or newly open
    // by default) opens or folds as it does by default; the old list of
    // open modules carries over: what was open stays open, and a module
    // open by default missing from it stays folded.
    void inspectorSectionFoldsKeepNewModulesDefaults() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        const QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        const auto build = [&timeline](const QStringList &selection) {
            auto inspector = std::make_unique<InspectorPanel>();
            inspector->setEffectCatalog(QJsonArray{});
            inspector->setState(timeline, selection);
            return inspector;
        };
        const auto opened = [](const InspectorPanel &inspector) {
            QStringList ids;
            for (const auto &entry : std::as_const(inspector.m_sections))
                if (entry.section->isOpen()) ids << entry.section->id();
            return ids;
        };
        // Left with only Crop open before Transform and Composite came open
        // by default: they open now, Crop stays open, and Anchor folded.
        QSettings().setValue(QStringLiteral("editor/inspector/sections/video"), QStringList{QStringLiteral("crop")});
        {
            const auto inspector = build({QStringLiteral("v")});
            QCOMPARE(opened(*inspector), (QStringList{"transform", "compositing", "crop"}));
        }
        // Tracks, open by default and missing from the sequence's list, was
        // folded by hand, and stays so.
        QSettings().setValue(QStringLiteral("editor/inspector/sections/sequence"), QStringList{QStringLiteral("markers")});
        {
            const auto inspector = build({});
            QVERIFY(!inspector->section(QStringLiteral("tracks"))->isOpen());
            QVERIFY(inspector->section(QStringLiteral("markers"))->isOpen());
        }
        // A fold is kept by id; a module never seen opens as it does by
        // default, though others of its kind were folded by hand.
        QSettings().remove(QStringLiteral("editor/inspector"));
        QSettings().setValue(QStringLiteral("editor/inspector/folds/video"),
                             QVariantMap{{QStringLiteral("crop"), true}, {QStringLiteral("compositing"), false}});
        {
            const auto inspector = build({QStringLiteral("v")});
            QCOMPARE(opened(*inspector), (QStringList{"transform", "crop"}));
            InspectorSection *transform = inspector->section(QStringLiteral("transform"));
            transform->setOpen(false);
            emit transform->toggled(false, Qt::NoModifier);
        }
        const QVariantMap folds = QSettings().value(QStringLiteral("editor/inspector/folds/video")).toMap();
        QCOMPARE(folds.value(QStringLiteral("transform")), QVariant(false));
        QCOMPARE(folds.value(QStringLiteral("crop")), QVariant(true));
        QVERIFY(!QSettings().contains(QStringLiteral("editor/inspector/sections/video")));
        QCOMPARE(opened(*build({QStringLiteral("v")})), (QStringList{"crop"}));
    }

    // Fit and Fill write the scale that fits or fills the frame, centred;
    // speed is a percentage, and Reverse its sign; a title's Place sets y.
    void inspectorPresetsWriteTheirValues() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, const QJsonObject &props, double speed, bool reverse) {
            QJsonObject clip{{"id", "v"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                             {"in", 0.0}, {"out", 2.0}, {"speed", speed}, {"props", props}};
            if (reverse) clip.insert("reverse", true);
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()}, {"generators", catalogGenerators()},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
                {"tracks", QJsonArray{
                    QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                    QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "t"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                    {"props", QJsonObject{{"text", "Lower"}}}}}}}}}}}});
        };
        snapshot(1, {}, 1.0, false);
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        auto *fit = inspector->findChild<KeyBank *>(QStringLiteral("fitFill"));
        QVERIFY(fit);
        QCOMPARE(fit->current(), 0);
        emit fit->activated(1);
        QJsonObject props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY2(std::abs(props.value("scale").toDouble() - 3.1605) < 0.001, qPrintable(QString::number(props.value("scale").toDouble())));
        QVERIFY(props.contains("x") && props.value("x").isNull() && props.contains("y") && props.value("y").isNull());
        snapshot(2, QJsonObject{{"scale", 3.1605}}, 1.0, false);
        QCOMPARE(fit->current(), 1);
        emit fit->activated(0);
        props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY(props.value("scale").isNull() && props.value("x").isNull());
        snapshot(3, QJsonObject{{"scale", 3.1605}, {"x", 0.1}}, 1.0, false);
        QCOMPARE(fit->current(), -1);
        // Speed: 200% is a ratio of 2; Reverse its sign; presets keep the direction.
        ScrubField *speed = inspector->findChild<ScrubField *>(QStringLiteral("speed"));
        auto *reverse = inspector->findChild<GlyphButton *>(QStringLiteral("speedReverse"));
        QVERIFY(speed && reverse);
        QCOMPARE(speed->text(), QStringLiteral("100"));
        // Off, Reverse is a quiet outline in the secondary ink, no disc.
        QVERIFY(!reverse->isChecked());
        QCOMPARE(reverse->inkColor(), editorTheme().textSecondary);
        QVERIFY(!reverse->discColor().isValid());
        speed->commitText(QStringLiteral("200"));
        QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("speed"));
        QCOMPARE(op.value("speed").toString(), QStringLiteral("2"));
        snapshot(4, {}, 2.0, false);
        reverse->click();
        QCOMPARE(takeOps(window).first().toObject().value("speed").toString(), QStringLiteral("-2"));
        snapshot(5, {}, 2.0, true);
        QCOMPARE(speed->text(), QString(QChar(0x2212)) + QStringLiteral("200"));
        QVERIFY(reverse->isChecked());
        // On: full ink on the neutral pressed disc, no purple wash.
        QCOMPARE(reverse->inkColor(), editorTheme().text);
        QCOMPARE(reverse->discColor(), editorTheme().pressed);
        QVERIFY(reverse->discColor() != editorTheme().accentTint);
        inspector->findChild<QAction *>(QStringLiteral("speed50"))->trigger();
        QCOMPARE(takeOps(window).first().toObject().value("speed").toString(), QStringLiteral("-0.5"));
        // A title's Place: Top is y −0.30.
        window.selectClip(QStringLiteral("t"));
        auto *place = inspector->findChild<SegmentedField *>(QStringLiteral("titlePlace"));
        QVERIFY(place);
        QCOMPARE(place->current(), 1);
        emit place->activated(0);
        QVERIFY(std::abs(takeOps(window).first().toObject().value("props").toObject().value("y").toDouble() + 0.30) < 1e-9);
        auto *align = inspector->findChild<SegmentedField *>(QStringLiteral("titleAlign"));
        QVERIFY(align);
        QCOMPARE(align->current(), 1);
        emit align->activated(0);
        QCOMPARE(takeOps(window).first().toObject().value("props").toObject().value("align").toString(), QStringLiteral("left"));
    }

    // Start, End and Duration are timecode fields on the sequence's rate:
    // Start moves the clip, End and Duration trim its out point; Source In
    // slips it on the media's rate. Typed timecode may be short, relative,
    // or seconds. Drop-frame rates show drop-frame timecode.
    void inspectorTimingSendsTimecodeEdits() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 20}, {"timeline", QJsonObject{
            {"fps", 24}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "media/fisher-close.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}, {"fps", 24}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 6.0}, {"duration", 5.0},
                            {"startFrame", 144}, {"durationFrames", 120}, {"in", 2.0}, {"out", 7.0}, {"speed", 1.0}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        ScrubField *start = inspector->findChild<ScrubField *>(QStringLiteral("timing_start"));
        ScrubField *end = inspector->findChild<ScrubField *>(QStringLiteral("timing_end"));
        ScrubField *duration = inspector->findChild<ScrubField *>(QStringLiteral("timing_duration"));
        ScrubField *sourceIn = inspector->findChild<ScrubField *>(QStringLiteral("source_in"));
        QVERIFY(start && end && duration && sourceIn);
        QCOMPARE(start->text(), QStringLiteral("00:00:06:00"));
        QCOMPARE(end->text(), QStringLiteral("00:00:11:00"));
        QCOMPARE(duration->text(), QStringLiteral("00:00:05:00"));
        QCOMPARE(sourceIn->text(), QStringLiteral("00:00:02:00"));
        // Five spin boxes became three timecode fields: no spin boxes at all.
        QVERIFY(inspector->findChildren<QDoubleSpinBox *>().isEmpty());
        const auto edit = [&window] { return takeOps(window).first().toObject(); };
        start->commitText(QStringLiteral("00:00:07:00"));
        QJsonObject op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("move"));
        QCOMPARE(op.value("frames").toInt(), 24);
        start->setValue(144);
        start->commitText(QStringLiteral("+12"));
        QCOMPARE(edit().value("frames").toInt(), 12);
        end->commitText(QStringLiteral("10:00"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op.value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(op.value("frames").toInt(), -24);
        duration->commitText(QStringLiteral("4s"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op.value("frames").toInt(), -24);
        sourceIn->commitText(QStringLiteral("00:00:03:00"));
        op = edit();
        QCOMPARE(op.value("op").toString(), QStringLiteral("slip"));
        QCOMPARE(op.value("frames").toInt(), 24);
        // 29.97 drop-frame.
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 120}, {"timeline", QJsonObject{
            {"fps", 30000.0 / 1001.0}, {"dropFrame", true},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 60.06}, {"duration", 5.0},
                            {"startFrame", 1800}, {"durationFrames", 150}}}}}}}}}});
        start = inspector->findChild<ScrubField *>(QStringLiteral("timing_start"));
        QCOMPARE(start->text(), QStringLiteral("00:01:00;02"));
        QCOMPARE(frameTimecode(17982, 30000.0 / 1001.0, true), QStringLiteral("00:10:00;00"));
        QCOMPARE(parseTimecode(QStringLiteral("00:01:00;02"), 30000.0 / 1001.0, true, nullptr).value(), qint64(1800));
        QCOMPARE(parseTimecode(QStringLiteral("7:00"), 24, false, nullptr).value(), qint64(168));
        bool relative = false;
        QCOMPARE(parseTimecode(QStringLiteral("-1:00"), 24, false, &relative).value(), qint64(-24));
        QVERIFY(relative);
    }

    static QString plainText(const QString &html) {
        static const QRegularExpression tags(QStringLiteral("<[^>]*>"));
        QString text = html;
        return text.remove(tags).replace(QStringLiteral("&amp;"), QStringLiteral("&"));
    }

    // The header never shows a stand-in name: a clip's own name, else its
    // media's file name, else what generates it, else where it is.
    void inspectorNamesEveryClip() {
        InspectorPanel inspector;
        const auto timeline = [](const QJsonObject &clip, const QJsonObject &source) {
            return QJsonObject{{"fps", 24}, {"width", 1920}, {"height", 1080}, {"sources", QJsonArray{source}},
                               {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                                                     QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}};
        };
        QJsonObject clip{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}};
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"width", 1920}, {"height", 1080}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Clip on V1"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"path", "media/harbor-wide.mov"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("harbor-wide.mov"));
        clip.insert(QStringLiteral("name"), QStringLiteral("Opening wide"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}, {"path", "media/harbor-wide.mov"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Opening wide"));
        // A stand-in from an older editor gives way too.
        clip.insert(QStringLiteral("name"), QStringLiteral("Untitled media"));
        inspector.setState(timeline(clip, QJsonObject{{"id", "m1"}}), {QStringLiteral("c1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Clip on V1"));
        const QJsonObject title{{"id", "t1"}, {"kind", "generator"}, {"fx", "title"}, {"timelineStart", 0.0}, {"duration", 2.0}};
        inspector.setState(timeline(title, QJsonObject{}), {QStringLiteral("t1")});
        QCOMPARE(inspector.nameText(), QStringLiteral("Title"));
    }

    // Selecting a clip shows what it is: a thumbnail, its name, its kind
    // in the clip's own timeline colour, its track, and its Start, End and
    // Duration in timecode.
    void inspectorShowsWhatEachClipIs() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        InspectorPanel *inspector = window.m_inspector;
        const EditorTheme &theme = editorTheme();
        const struct {
            const char *id;
            const char *kind;
            const char *track;
            ClipKind color;
            const char *start;
            const char *duration;
        } cases[] = {
            {"v", "Video", "V1 Video 1 · linked A1", ClipKind::Video, "00:00:06:00", "00:00:05:00"},
            {"n", "Sequence", "V1 Video 1", ClipKind::Video, "00:00:11:00", "00:00:04:00"},
            {"t", "Title", "V2 Titles", ClipKind::Generator, "00:00:01:00", "00:00:04:00"},
            {"s", "Solid", "V2 Titles", ClipKind::Generator, "00:00:05:00", "00:00:02:00"},
            {"adj", "Adjustment", "V2 Titles", ClipKind::Generator, "00:00:08:00", "00:00:03:00"},
            // As the timeline's header names it: the subtitle track is the
            // third picture track, V3.
            {"cap", "Caption", "V3 Captions", ClipKind::Caption, "00:00:02:00", "00:00:04:00"},
            {"a", "Audio", "A1 Dialogue · linked V1", ClipKind::Audio, "00:00:06:00", "00:00:05:00"},
            {"music", "Audio", "A2 Music", ClipKind::Music, "00:00:00:00", "00:00:18:00"},
        };
        for (const auto &expected : cases) {
            const QString id = QString::fromLatin1(expected.id);
            window.selectClip(id);
            const QString kind = plainText(inspector->kindText());
            QCOMPARE(kind, QStringLiteral("%1 · %2").arg(QString::fromUtf8(expected.kind), QString::fromUtf8(expected.track)));
            QCOMPARE(inspector->m_kindSquare->color().rgb(), theme.clip(expected.color).header.rgb());
            QVERIFY(inspector->m_kindSquare->isVisibleTo(inspector));
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_start"))->text(), QString::fromLatin1(expected.start));
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_duration"))->text(), QString::fromLatin1(expected.duration));
            const QList<QImage> thumbs = inspector->m_thumbnail->images();
            QCOMPARE(thumbs.size(), 1);
            QVERIFY2(!thumbs.first().isNull(), expected.id);
            QVERIFY(!inspector->nameText().isEmpty());
        }
        // The video's thumbnail is its picture; the solid's its colour.
        window.selectClip(QStringLiteral("s"));
        const QImage solid = inspector->m_thumbnail->images().first();
        QCOMPARE(solid.pixelColor(solid.width() / 2, solid.height() / 2).rgb(), QColor(0x33, 0x55, 0x77).rgb());
        // The adjustment says what it is for; the caption how to change it.
        window.selectClip(QStringLiteral("adj"));
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Applies its effects to every layer beneath it."));
        QVERIFY(!inspector->m_note->isHidden());
        // Its effects come first, and open.
        QVERIFY(inspector->section(QStringLiteral("effects"))->isOpen());
        window.selectClip(QStringLiteral("cap"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("captionText"))->text(), QStringLiteral("We open on the harbor at dawn."));
        // Ask Kerf to change it: a link (pointing hand, no bezel), not grey text.
        auto *captionAsk = inspector->findChild<QPushButton *>(QStringLiteral("captionHint"));
        QVERIFY(captionAsk && captionAsk->property("inspectorLink").toBool());
        QCOMPARE(captionAsk->cursor().shape(), Qt::PointingHandCursor);
        // Its one module holds the text.
        QCOMPARE(inspector->m_sections.size(), 1);
        QCOMPARE(inspector->m_sections.first().section->id(), QStringLiteral("caption"));
        // The video's Source section: its file and format, as plain text.
        window.selectClip(QStringLiteral("v"));
        QCOMPARE(inspector->section(QStringLiteral("source"))->summary(), QStringLiteral("1920 × 1080 · 24 fps"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sourceFile"))->text(), QStringLiteral("fisher-close.mov"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sourceAudio"))->text(), QStringLiteral("2 ch · 48 kHz"));
        // Its linked sound's volume and pan, edited on the linked clip.
        InspectorSection *linked = inspector->section(QStringLiteral("linkedAudio"));
        QVERIFY(linked);
        QCOMPARE(linked->title(), QStringLiteral("Audio · A1"));
        inspector->findChild<ScrubField *>(QStringLiteral("linked_gain"))->commitText(QStringLiteral("-8"));
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("clip").toString(), QStringLiteral("a"));
        QCOMPARE(op.value("props").toObject().value("gain").toDouble(), -8.0);
        // The music's folded volume reads in dB.
        window.selectClip(QStringLiteral("music"));
        QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("prop_gain"))->text(), QString(QChar(0x2212)) + QStringLiteral("8.0"));
        // Its thumbnail is its real waveform, as the bin's tile: the glyph
        // ink in the band the wave fills, the tile's colour above it.
        {
            const QImage wave = inspector->m_thumbnail->images().first();
            QCOMPARE(wave.pixelColor(64, 4).rgb(), theme.audioTile.rgb());
            int inked = 0;
            for (int x = 0; x < wave.width(); ++x) inked += wave.pixelColor(x, 36).rgb() == theme.audioTileGlyph.rgb() ? 1 : 0;
            QVERIFY2(inked > wave.width() / 2, qPrintable(QString::number(inked)));
        }
        // A sound clip carrying no effects has no Effects section: no effect
        // in this version of Kerf applies to sound.
        QVERIFY(!inspector->section(QStringLiteral("effects")));
    }

    // The name edits in place: Return renames the clip (set {name}), Esc
    // keeps it. ⋯ ▸ Ask Kerf opens the conversation about the clip, and the
    // composer names the selected clip.
    void inspectorRenamesAndAsksKerf() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        NameLabel *name = inspector->m_name;
        QCOMPARE(name->name(), QStringLiteral("fisher-close.mov"));
        QVERIFY(name->isEditable());
        QMouseEvent doubleClick(QEvent::MouseButtonDblClick, QPointF(4, 4), name->mapToGlobal(QPointF(4, 4)), Qt::LeftButton,
                                Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(name, &doubleClick);
        QVERIFY(name->editor() && name->editor()->isVisibleTo(name));
        QCOMPARE(name->editor()->text(), QStringLiteral("fisher-close.mov"));
        name->editor()->setText(QStringLiteral("Fisher close-up"));
        key(name->editor(), Qt::Key_Return);
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("v"));
        QCOMPARE(op.value("name").toString(), QStringLiteral("Fisher close-up"));
        QVERIFY(!op.contains("props"));
        name->beginEditing();
        name->editor()->setText(QStringLiteral("Nope"));
        key(name->editor(), Qt::Key_Escape);
        QVERIFY(takeOps(window).isEmpty());
        QVERIFY(!name->editor()->isVisibleTo(name));
        QCOMPARE(name->name(), QStringLiteral("fisher-close.mov"));
        // The composer names the selected clip.
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf about fisher-close.mov…"));
        // ⋯ ▸ Ask Kerf about This Clip: the conversation, open, about it.
        window.setChatOpen(false);
        QAction *ask = inspector->m_moreMenu->findChild<QAction *>(QStringLiteral("inspectorAsk"));
        QVERIFY(ask);
        ask->trigger();
        QVERIFY(window.chatOpen());
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("@fisher-close.mov "));
        window.selectClip(QString());
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Describe your next edit…"));
        // A sequence is not renamed here.
        QVERIFY(!name->isEditable());
    }

    // With nothing selected the Inspector shows the sequence: its format,
    // its tracks and its markers, as text, with only the frame size to
    // change; and a quiet hint.
    void inspectorShowsTheSequenceWithNothingSelected() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject snapshot = everyKindSnapshot(1);
        window.applySnapshot(snapshot);
        window.selectClip(QString());
        InspectorPanel *inspector = window.m_inspector;
        QCOMPARE(inspector->nameText(), QStringLiteral("Harbor Teaser"));
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("Sequence · 1080 × 1920 · 24 fps"));
        QVERIFY(!inspector->m_kindSquare->isVisibleTo(inspector));
        QVERIFY(inspector->findChildren<ScrubField *>().isEmpty());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("timing_start"))->text(), QStringLiteral("00:00:00:00"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("timing_duration"))->text(), QStringLiteral("00:00:18:00"));
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Select a clip to adjust it."));
        // The hint is its own line: 8 points under the timing row.
        {
            QLayout *header = inspector->m_header->layout();
            const int tall = header->heightForWidth(300) > 0 ? header->heightForWidth(300) : header->sizeHint().height();
            header->setGeometry(QRect(0, 0, 300, tall));
            const int below = inspector->m_note->geometry().top() + inspector->m_note->contentsMargins().top() - inspector->m_timing->geometry().bottom() - 1;
            QVERIFY2(below >= 8, qPrintable(QString::number(below)));
            const int above = tall - inspector->m_note->geometry().bottom() - 1 + inspector->m_note->contentsMargins().bottom();
            QVERIFY2(above >= 8, qPrintable(QString::number(above)));
        }
        InspectorSection *format = inspector->section(QStringLiteral("format"));
        InspectorSection *tracks = inspector->section(QStringLiteral("tracks"));
        InspectorSection *markers = inspector->section(QStringLiteral("markers"));
        QVERIFY(format && tracks && markers);
        QVERIFY(format->isOpen() && tracks->isOpen() && !markers->isOpen());
        const int trackCount = int(snapshot.value("timeline").toObject().value("tracks").toArray().size());
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("sequenceTrack")).size(), trackCount);
        // Each track's square in its timeline header's colour: a picture
        // track of titles is a titles track, as the timeline draws it.
        {
            const QJsonArray trackList = snapshot.value("timeline").toObject().value("tracks").toArray();
            const auto lines = inspector->findChildren<QWidget *>(QStringLiteral("sequenceTrack"));
            bool titles = false;
            for (int index = 0; index < trackList.size(); ++index) {
                const QJsonObject track = trackList.at(index).toObject();
                const ClipKind kind = trackKind(track);
                QCOMPARE(lines.at(index)->findChild<KindSquare *>()->color(), editorTheme().clip(kind).header);
                titles = titles || (kind == ClipKind::Generator && !track.value("type").toString().contains(QStringLiteral("title"), Qt::CaseInsensitive));
            }
            QVERIFY2(titles, "a picture track holding titles");
        }
        QCOMPARE(tracks->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QString::number(trackCount));
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("sequenceMarker")).size(), 2);
        QCOMPARE(markers->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("2"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sequenceRate"))->text(), QStringLiteral("24 fps"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("sequenceAudio"))->text(), QStringLiteral("48 kHz"));
        // Only the frame size can change; no clip fields.
        const auto popups = inspector->findChildren<QComboBox *>();
        QCOMPARE(popups.size(), 1);
        QComboBox *size = popups.first();
        QCOMPARE(size->objectName(), QStringLiteral("frameSize"));
        // Written as the header writes it (thin spaces round the ×).
        QCOMPARE(size->currentText(), QStringLiteral("9:16 · 1080\u2009×\u20091920"));
        QVERIFY(inspector->findChildren<QLineEdit *>().size() <= 1); // the name's editor, hidden
        QVERIFY(inspector->findChildren<QPlainTextEdit *>().isEmpty());
        QVERIFY(inspector->findChildren<InspectorSwitch *>().isEmpty());
        size->setCurrentIndex(0);
        emit size->activated(0);
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set-sequence"));
        QCOMPARE(op.value("width").toInt(), 1920);
        QCOMPARE(op.value("height").toInt(), 1080);
    }

    // Several clips: "3 clips", what they share, "Mixed" where they differ;
    // a drag moves each by the same amount and typing sets each to the same
    // value, either way in one edit. Clips of different kinds share only
    // their timing.
    void inspectorShowsWhatSeveralClipsShare() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto clip = [](const char *id, double start, const QJsonObject &props) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", start}, {"duration", 2.0},
                               {"startFrame", std::llround(start * 24)}, {"durationFrames", 48}, {"props", props}};
        };
        QJsonObject first = clip("v1", 0.0, QJsonObject{{"opacity", 0.5}});
        first.insert("effects", QJsonArray{QJsonObject{{"fx", "blur.gaussian"}, {"enabled", true}, {"params", QJsonObject{}}}});
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    first, clip("v2", 2.0, QJsonObject{{"opacity", 0.5}, {"x", 0.1}}), clip("v3", 4.0, QJsonObject{{"opacity", 1.0}})}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "a1"}, {"timelineStart", 0.0}, {"duration", 2.0}, {"startFrame", 0}, {"durationFrames", 48}}}}}}}}}});
        InspectorPanel *inspector = window.m_inspector;
        inspector->setSelection({QStringLiteral("v1"), QStringLiteral("v2"), QStringLiteral("v3")});
        QCOMPARE(inspector->nameText(), QStringLiteral("3 clips"));
        QVERIFY(!inspector->m_name->isEditable());
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("Video · V1"));
        QCOMPARE(inspector->m_thumbnail->images().size(), 3);
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        ScrubField *x = inspector->findChild<ScrubField *>(QStringLiteral("prop_x"));
        QVERIFY(opacity && x);
        QVERIFY(opacity->isMixed() && x->isMixed());
        QCOMPARE(opacity->text(), QStringLiteral("Mixed"));
        QVERIFY(inspector->findChild<InspectorSlider *>(QStringLiteral("slider_opacity"))->m_mixed);
        QCOMPARE(inspector->section(QStringLiteral("effects"))->summary(), QStringLiteral("On 1 of 3 clips"));
        // A drag of 20 points moves each clip 10 px, in one edit.
        x->resize(78, ScrubField::kHeight);
        mouse(x, QEvent::MouseButtonPress, QPointF(20, 12));
        mouse(x, QEvent::MouseMove, QPointF(40, 12));
        mouse(x, QEvent::MouseButtonRelease, QPointF(40, 12));
        QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        const QList<double> before{0.0, 192.0, 0.0};
        for (int index = 0; index < 3; ++index) {
            const QJsonObject op = ops.at(index).toObject();
            QCOMPARE(op.value("clip").toString(), QStringLiteral("v%1").arg(index + 1));
            QVERIFY(std::abs(op.value("props").toObject().value("x").toDouble() * 1920.0 - (before.at(index) + 10.0)) < 1e-6);
        }
        // Typing sets each to the value.
        x->commitText(QStringLiteral("50"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        for (const QJsonValue &op : ops) QVERIFY(std::abs(op.toObject().value("props").toObject().value("x").toDouble() * 1920.0 - 50.0) < 1e-6);
        // The timing moves them all.
        inspector->findChild<ScrubField *>(QStringLiteral("timing_start"))->commitText(QStringLiteral("+12"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 3);
        for (const QJsonValue &op : ops) {
            QCOMPARE(op.toObject().value("op").toString(), QStringLiteral("move"));
            QCOMPARE(op.toObject().value("frames").toInt(), 12);
        }
        // End is the span's end: a new End moves it by trimming only the
        // clip that ends there (v3), so no gap opens after v1 or v2.
        ScrubField *spanEnd = inspector->findChild<ScrubField *>(QStringLiteral("timing_end"));
        QCOMPARE(spanEnd->value(), 144.0);
        spanEnd->commitText(QStringLiteral("00:00:05:00"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.at(0).toObject().value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(ops.at(0).toObject().value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(ops.at(0).toObject().value("clip").toString(), QStringLiteral("v3"));
        QCOMPARE(ops.at(0).toObject().value("frames").toInt(), -24);
        spanEnd->commitText(QStringLiteral("+12"));
        ops = takeOps(window);
        QCOMPARE(ops.size(), 1);
        QCOMPARE(ops.at(0).toObject().value("clip").toString(), QStringLiteral("v3"));
        QCOMPARE(ops.at(0).toObject().value("frames").toInt(), 12);
        // The timing row keeps one alignment, timecode and "Mixed" alike.
        for (ScrubField *field : std::as_const(inspector->m_timingFields)) QCOMPARE(field->valueAlignment(), Qt::Alignment(Qt::AlignLeft));
        // Different kinds: the timing only, and why.
        inspector->setSelection({QStringLiteral("v1"), QStringLiteral("a1")});
        QCOMPARE(plainText(inspector->kindText()), QStringLiteral("1 Audio, 1 Video"));
        QVERIFY(inspector->findChildren<InspectorRow *>().isEmpty());
        QCOMPARE(inspector->m_timingFields.size(), 3);
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Video and audio clips have no properties in common."));
    }

    // Selecting another clip of the same shape only changes values: the
    // same row widgets, open sections and scroll offset.
    void inspectorReusesRowsForClipsOfOneShape() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonArray stack;
        for (int index = 0; index < 10; ++index)
            stack.append(QJsonObject{{"fx", index % 2 ? "color.basic" : "blur.gaussian"}, {"enabled", true}, {"params", QJsonObject{}}});
        const auto clip = [&stack](const char *id, double start, double opacity) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", start}, {"duration", 2.0},
                               {"props", QJsonObject{{"opacity", opacity}}}, {"effects", stack}};
        };
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("c1", 0, 0.5), clip("c2", 2, 0.25)}}}}}}}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("c1"));
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->isVisible());
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QCOMPARE(opacity->text(), QStringLiteral("50"));
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        inspector->section(QStringLiteral("crop"))->setOpen(true);
        QCoreApplication::processEvents();
        QScrollBar *scroll = inspector->scrollArea()->verticalScrollBar();
        // The body scrolls with its scroll bar hidden (mockup-spec 5): it
        // takes no width; the Inspector still marks when the pointer is over it.
        {
            QCOMPARE(scroll->sizeHint().width(), 0);
            QVERIFY(!inspector->scrollArea()->property("active").toBool());
            QEnterEvent enter(QPointF(10, 10), QPointF(10, 10), QPointF(10, 10));
            QCoreApplication::sendEvent(inspector, &enter);
            QVERIFY(inspector->scrollArea()->property("active").toBool());
            QEvent leave(QEvent::Leave);
            QCoreApplication::sendEvent(inspector, &leave);
            QVERIFY(!inspector->scrollArea()->property("active").toBool());
        }
        QVERIFY(scroll->maximum() > 60);
        scroll->setValue(60);
        // From the click to the painted values (the panel repainted at once,
        // not when the event loop gets to it).
        const auto timedSwitch = [&](const char *id) {
            QElapsedTimer timer;
            timer.start();
            window.clickSelectClip(QString::fromLatin1(id));
            inspector->repaint();
            return timer.nsecsElapsed() / 1.0e6;
        };
        const double elapsed = timedSwitch("c2");
        QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity")), opacity);
        QCOMPARE(opacity->text(), QStringLiteral("25"));
        QVERIFY(inspector->section(QStringLiteral("effects"))->isOpen() && inspector->section(QStringLiteral("crop"))->isOpen());
        QCoreApplication::processEvents();
        QCOMPARE(scroll->value(), 60);
        // Values only: well under a frame, even with ten effects. The median
        // of a run of switches back and forth, so one slow scheduling slice
        // on a shared CI machine does not decide it. The target is 8 ms; the
        // check allows four times that, as CI builds the tests unoptimized on
        // shared runners (7.9-9.9 ms measured unoptimized on an M-series Mac,
        // offscreen, but 17-21 ms on GitHub's macOS runner; see the README).
        QList<double> times{elapsed};
        for (int index = 0; index < 8; ++index) times << timedSwitch(index % 2 ? "c2" : "c1");
        std::sort(times.begin(), times.end());
        const double median = times.at(times.size() / 2);
        qInfo("Inspector: same-shape clip switch, click to painted values: median %.2f ms, fastest %.2f ms (10 effects)", median,
              times.first());
        QVERIFY2(median < 32.0, qPrintable(QStringLiteral("%1 ms").arg(median)));
        QCOMPARE(scroll->value(), 60);
        // The header's hairline shows while the body is scrolled under it,
        // laid over the body's top edge, so the body does not move for it.
        QVERIFY(inspector->m_scrollEdge->isVisible());
        QCOMPARE(inspector->m_scrollEdge->parentWidget(), inspector->m_scroll);
        QCOMPARE(inspector->m_scrollEdge->geometry(), QRect(0, 0, inspector->m_scroll->width(), 1));
        scroll->setValue(0);
        QVERIFY(!inspector->m_scrollEdge->isVisible());
    }

    // A clip of a shape seen a moment ago shows the same rows again, with
    // its own values: going back and forth between a title and a video
    // clip leaves one body in the column, never a stale one.
    void inspectorShowsAShapeAgainWithItsValues() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        InspectorPanel *inspector = window.m_inspector;
        const auto visibleRows = [inspector] {
            QStringList labels;
            for (InspectorRow *row : inspector->findChildren<InspectorRow *>())
                if (row->isVisibleTo(inspector)) labels << row->label()->text();
            return labels;
        };
        window.clickSelectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        const QStringList titleRows = visibleRows();
        const QString titleName = inspector->nameText();
        QVERIFY(!titleRows.isEmpty());
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        const QStringList videoRows = visibleRows();
        QVERIFY(videoRows != titleRows);
        for (int round = 0; round < 2; ++round) {
            window.clickSelectClip(QStringLiteral("t"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), titleRows);
            QCOMPARE(inspector->nameText(), titleName);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
            window.clickSelectClip(QStringLiteral("v"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), videoRows);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
        }
        // A font or screen density change (EditorAppearance::changed)
        // builds every body again.
        emit EditorAppearance::instance()->changed();
        window.clickSelectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        QCOMPARE(visibleRows(), titleRows);
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
    }

    // The Inspector for every kind of clip, nothing selected and several
    // clips, in both appearances, and the whole editor with a clip clicked
    // in 9:16 and 16:9. With KERF_SCREENSHOT_DIR set, each is saved as
    // inspector-<state>-<scheme>.png and editor-<format>-inspector-<scheme>.png.
    void inspectorRendersEveryState() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto save = [&shots](const QImage &image, const QString &name) {
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        // The showcase with the close-up graded, scaled up and masked, and
        // more kinds of clip.
        const auto snapshotFor = [](const QSize &canvas) {
            QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
            snapshot.remove(QStringLiteral("pendingProposal"));
            snapshot.remove(QStringLiteral("proposalTimeline"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), canvas.width());
            timeline.insert(QStringLiteral("height"), canvas.height());
            timeline.insert(QStringLiteral("sampleRate"), 48000);
            timeline.insert(QStringLiteral("properties"), catalogProperties());
            timeline.insert(QStringLiteral("generators"), catalogGenerators());
            timeline.insert(QStringLiteral("maskModes"), QJsonArray{"add", "subtract", "intersect", "none"});
            timeline.insert(QStringLiteral("maskProperties"), QJsonArray{
                QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0.0}, {"min", 0.0}, {"max", 10000.0}, {"unit", "pixels"}},
                QJsonObject{{"name", "opacity"}, {"type", "number"}, {"default", 1.0}, {"min", 0.0}, {"max", 1.0}},
                QJsonObject{{"name", "expansion"}, {"type", "number"}, {"default", 0.0}, {"min", -10000.0}, {"max", 10000.0}, {"unit", "pixels"}}});
            QJsonArray tracks = timeline.value(QStringLiteral("tracks")).toArray();
            const auto edit = [&tracks](int track, int index, const std::function<void(QJsonObject &)> &change) {
                QJsonObject t = tracks.at(track).toObject();
                QJsonArray clips = t.value(QStringLiteral("clips")).toArray();
                QJsonObject clip = clips.at(index).toObject();
                change(clip);
                clips.replace(index, clip);
                t.insert(QStringLiteral("clips"), clips);
                tracks.replace(track, t);
            };
            edit(0, 1, [](QJsonObject &clip) {
                clip.insert("startFrame", 144);
                clip.insert("durationFrames", 120);
                clip.insert("speed", 1.0);
                clip.insert("link", "L2");
                clip.insert("props", QJsonObject{{"scale", 1.12}, {"y", -0.025}, {"crop_b", 0.0556}});
                clip.insert("masks", QJsonArray{QJsonObject{{"name", "vignette"}, {"ellipse", QJsonArray{0.5, 0.5, 0.4, 0.4}}, {"mode", "add"},
                                                            {"inverted", false}, {"params", QJsonObject{{"feather", 40.0}}}}});
                clip.insert("effects", QJsonArray{
                    QJsonObject{{"fx", "color.basic"}, {"enabled", true},
                                {"params", QJsonObject{{"exposure", 0.4}, {"contrast", 1.1}, {"saturation", 0.92}, {"temperature", 350.0}}}},
                    QJsonObject{{"fx", "blur.gaussian"}, {"enabled", false}, {"params", QJsonObject{{"radius", 8.0}}}}});
            });
            edit(1, 0, [](QJsonObject &clip) {
                clip.insert("props", QJsonObject{{"text", "Harbor Teaser"}, {"size", 0.06}, {"y", 0.28}});
            });
            QJsonObject titles = tracks.at(1).toObject();
            QJsonArray titleClips = titles.value(QStringLiteral("clips")).toArray();
            titleClips.append(QJsonObject{{"id", "adj"}, {"kind", "generator"}, {"fx", "adjustment"}, {"name", "Warm grade"},
                                          {"timelineStart", 6.0}, {"duration", 5.0},
                                          {"effects", QJsonArray{QJsonObject{{"fx", "color.basic"}, {"enabled", true},
                                                                             {"params", QJsonObject{{"temperature", 400.0}, {"saturation", 1.1}}}}}}});
            titles.insert(QStringLiteral("clips"), titleClips);
            tracks.replace(1, titles);
            edit(3, 1, [](QJsonObject &clip) { clip.insert("link", "L2"); clip.insert("speed", 1.0); });
            edit(4, 0, [](QJsonObject &clip) {
                clip.insert("speed", 1.0);
                clip.insert("props", QJsonObject{{"gain", -8.0}, {"pan", -0.4}});
            });
            timeline.insert(QStringLiteral("tracks"), tracks);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            return snapshot;
        };
        {
            const EditorTheme &theme = editorTheme();
            const QString name = QStringLiteral("dark");
            for (const QSize canvas : {QSize(1080, 1920), QSize(1920, 1080)}) {
                const bool tall = canvas.height() > canvas.width();
                QSettings().remove(QStringLiteral("workspace"));
                QSettings().remove(QStringLiteral("editor/inspector"));
                MainWindow window{QString(), QString()};
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshotFor(canvas));
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                // The playhead in the close-up, where the frame shown is: the
                // time on the picture and the timeline's playhead agree with it.
                window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 6.0}, {"duration", 18.0}});
                window.m_viewer->setFrame(testing::footageFrame(testing::Shot::FisherClose, canvas / 4), 6.0);
                QCOMPARE(window.m_timeline->m_playhead, 6.0);
                window.m_inspector->setProgramFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(160, 90)));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Warm up the close-up a little."));
                window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I raised the exposure and the temperature on fisher-close.mov."));
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                window.clickSelectClip(QStringLiteral("v1b"));
                QTest::qWait(30);
                ads::CDockWidget *panel = window.m_workspace->panel(QStringLiteral("inspector"));
                QVERIFY(panel->isCurrentTab());
                InspectorPanel *inspector = window.m_inspector;
                // The chat is untouched beside the picture.
                QVERIFY(window.m_chat->isVisible());
                QCOMPARE(window.m_chat->geometry(), window.m_viewer->stageLayout().panel);
                QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf about fisher-close…"));
                const QImage editor = window.grab().toImage();
                // Amber is the lamps' alone: the close-up's changed values
                // light the Inspector's, and nothing outside it is amber.
                {
                    QImage outside = editor.copy();
                    QPainter cover(&outside);
                    cover.fillRect(QRect(panel->mapTo(&window, QPoint()), panel->size()), Qt::black);
                    cover.end();
                    QCOMPARE(amberPixels(outside), 0);
                }
                save(editor, QStringLiteral("editor-%1-inspector-%2").arg(tall ? QStringLiteral("9x16") : QStringLiteral("16x9"), name));
                if (!tall) continue;
                // Framed modules on the panel, the values in recessed wells.
                const auto grabPanel = [panel] { return panel->grab().toImage(); };
                {
                    const QImage image = grabPanel();
                    const qreal ratio = image.devicePixelRatio();
                    // An open module stands apart on the panel...
                    InspectorSection *transform = inspector->section(QStringLiteral("transform"));
                    const QPoint top = transform->mapTo(panel, QPoint(transform->width() / 2, 0));
                    QCOMPARE(image.pixelColor(top * ratio).rgb(), theme.moduleEdge.rgb());
                    QCOMPARE(image.pixelColor((top - QPoint(0, 1)) * ratio).rgb(), theme.panel.rgb());
                    // ...and the folded ones stand together as a rack, a
                    // hairline between Time and Crop, plate on either side.
                    InspectorSection *time = inspector->section(QStringLiteral("time"));
                    InspectorSection *crop = inspector->section(QStringLiteral("crop"));
                    QVERIFY(!time->isOpen() && !time->joinsAbove() && time->joinsBelow());
                    QVERIFY(crop->joinsAbove());
                    QCOMPARE(crop->y(), time->geometry().bottom() + 1);
                    const QPoint edge = crop->mapTo(panel, QPoint(crop->width() / 2, 0));
                    QCOMPARE(image.pixelColor(edge * ratio).rgb(), theme.moduleEdge.rgb());
                    QCOMPARE(image.pixelColor((edge - QPoint(0, 2)) * ratio).rgb(), theme.modulePlate.rgb());
                    ScrubField *scale = inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"));
                    const QPoint inside = scale->mapTo(panel, QPoint(3, scale->height() / 2));
                    QCOMPARE(image.pixelColor(inside * ratio).rgb(), theme.wellFill.rgb());
                    // The plates, not purple, are what the Inspector is made of.
                    QCOMPARE(dominantColor(image), theme.modulePlate.rgb());
                    // The moved close-up lights its modules' lamps (and its rows').
                    QCOMPARE(inspector->section(QStringLiteral("transform"))->lamp()->state(), LampState::Changed);
                    QVERIFY(qobject_cast<InspectorRow *>(scale->parentWidget())->lamp()->isVisible());
                    QCOMPARE(inspector->section(QStringLiteral("compositing"))->lamp()->state(), LampState::Off);
                }
                // The video, its Scale row under the pointer: ↺ and its changed value.
                auto *scaleRow = qobject_cast<InspectorRow *>(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale"))->parentWidget());
                {
                    QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), QPointF(4, 4));
                    QCoreApplication::sendEvent(scaleRow, &enter);
                    QCoreApplication::sendEvent(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale")), &enter);
                }
                QVERIFY(scaleRow->resetButton()->isVisible());
                save(grabPanel(), QStringLiteral("inspector-video-%1").arg(name));
                {
                    QEvent leave(QEvent::Leave);
                    QCoreApplication::sendEvent(scaleRow, &leave);
                    QCoreApplication::sendEvent(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale")), &leave);
                }
                // Folded, the Effects header in the 300pt column names what
                // is on in whole words, never cut ("…"), and counts the rest.
                {
                    InspectorSection *fx = inspector->section(QStringLiteral("effects"));
                    QVERIFY(!fx->isOpen());
                    QCoreApplication::processEvents();
                    auto *label = fx->findChild<QLabel *>(QStringLiteral("sectionSummary"));
                    const QString shown = fx->shownSummary();
                    QVERIFY2(!shown.isEmpty() && !shown.contains(QChar(0x2026)), qPrintable(shown));
                    QVERIFY2(label->fontMetrics().horizontalAdvance(shown) <= label->contentsRect().width(), qPrintable(shown));
                    QVERIFY(fx->summary().startsWith(QStringLiteral("Primary Colour Correction")));
                    QCOMPARE(label->toolTip(), fx->summary());
                }
                // Its effects: Color open with "6 more", the blur folded with its line.
                inspector->section(QStringLiteral("effects"))->setOpen(true);
                for (const char *section : {"crop", "anchor", "linkedAudio", "masks", "source"})
                    inspector->section(QString::fromLatin1(section))->setOpen(false);
                const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
                QCOMPARE(boxes.size(), 2);
                boxes.at(0)->findChild<GlyphButton *>(QStringLiteral("effectChevron"))->click();
                QCOMPARE(boxes.at(0)->findChild<QPushButton *>(QStringLiteral("effectMoreToggle"))->text(), QStringLiteral("6 more"));
                QCOMPARE(boxes.at(1)->findChild<QLabel *>(QStringLiteral("effectSummary"))->text(), QStringLiteral("Radius 8 px"));
                QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("effectParam_exposure"))->text(), QStringLiteral("+0.40"));
                QCoreApplication::processEvents();
                for (ScrubField *field : boxes.at(0)->findChildren<ScrubField *>())
                    if (field->isVisible()) QVERIFY2(field->showsWhole(), qPrintable(field->objectName()));
                QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("effectParam_contrast"))->text(), QStringLiteral("110"));
                QCoreApplication::processEvents();
                inspector->scrollArea()->ensureWidgetVisible(boxes.at(0), 0, 0);
                inspector->scrollArea()->verticalScrollBar()->setValue(inspector->section(QStringLiteral("effects"))->y() - 8);
                QCoreApplication::processEvents();
                save(grabPanel(), QStringLiteral("inspector-video-effects-%1").arg(name));
                inspector->section(QStringLiteral("effects"))->setOpen(false);
                // The other kinds.
                for (const auto &[clip, label] : {std::pair{"a2", "audio"}, std::pair{"t1", "title"}, std::pair{"adj", "adjustment"}, std::pair{"c1", "caption"}}) {
                    window.clickSelectClip(QString::fromLatin1(clip));
                    QCoreApplication::processEvents();
                    QVERIFY2(panel->isCurrentTab(), qPrintable(QStringLiteral("%1: %2").arg(QString::fromLatin1(clip),
                                                                 panel->dockAreaWidget()->currentDockWidget()->objectName())));
                    save(grabPanel(), QStringLiteral("inspector-%1-%2").arg(QString::fromLatin1(label), name));
                }
                // Nothing selected (with the Inspector's tab chosen by hand, so it stays).
                window.m_workspace->panel(QStringLiteral("project"))->setAsCurrentTab();
                panel->setAsCurrentTab();
                window.clickSelectClip(QString());
                QCoreApplication::processEvents();
                QVERIFY(panel->isCurrentTab());
                save(grabPanel(), QStringLiteral("inspector-nothing-%1").arg(name));
                // Several clips.
                inspector->setSelection({QStringLiteral("v1a"), QStringLiteral("v1b"), QStringLiteral("v1c")});
                QCoreApplication::processEvents();
                save(grabPanel(), QStringLiteral("inspector-multi-%1").arg(name));
                window.close();
            }
        }
    }

    // The Inspector in the hardware finish: framed modules with engraved
    // titles and lamps (the folded ones a rack), values in wells, Fit and
    // Fill and Flip as latching keys, a knob beside Rotation, the timing
    // on a display glass, and each effect's power key.
    void inspectorHardwareModules() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        // The video clip "v" of everyKindSnapshot, with `props` and `effects`.
        const QJsonObject base = everyKindSnapshot(1).value("timeline").toObject();
        const auto withProps = [&base](const QJsonObject &props, const QJsonArray &effects = {}) {
            QJsonObject timeline = base;
            QJsonArray tracks = timeline.value("tracks").toArray();
            QJsonObject video = tracks.at(0).toObject();
            QJsonArray clips = video.value("clips").toArray();
            QJsonObject clip = clips.at(0).toObject();
            clip.insert("props", props);
            clip.insert("effects", effects);
            clips.replace(0, clip);
            video.insert("clips", clips);
            tracks.replace(0, video);
            timeline.insert("tracks", tracks);
            return timeline;
        };
        InspectorPanel inspector;
        inspector.setEffectCatalog(QJsonArray{});
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        const auto lastProps = [&sent] { return sent.isEmpty() ? QJsonObject() : sent.last().first().toObject().value("props").toObject(); };
        inspector.setState(withProps({}), {QStringLiteral("v")});
        inspector.resize(300, 900);
        inspector.show();
        QVERIFY(QTest::qWaitForWindowExposed(&inspector));
        QCoreApplication::processEvents();

        // Every row is in a module: Transform and Composite open first, then
        // the folded ones, Time heading them, standing together as a rack.
        QStringList ids;
        for (const auto &entry : std::as_const(inspector.m_sections)) ids << entry.section->id();
        QCOMPARE(ids.mid(0, 4), (QStringList{"transform", "compositing", "time", "crop"}));
        for (InspectorRow *row : inspector.findChildren<InspectorRow *>())
            QVERIFY2(qobject_cast<InspectorSection *>(row->parentWidget()->parentWidget()), qPrintable(row->label()->text()));
        InspectorSection *transform = inspector.section(QStringLiteral("transform"));
        InspectorSection *compositing = inspector.section(QStringLiteral("compositing"));
        InspectorSection *time = inspector.section(QStringLiteral("time"));
        InspectorSection *crop = inspector.section(QStringLiteral("crop"));
        QVERIFY(transform->isOpen() && compositing->isOpen() && !time->isOpen() && !crop->isOpen());
        QVERIFY(!transform->joinsAbove() && !transform->joinsBelow() && !compositing->joinsBelow());
        QVERIFY(!time->joinsAbove() && time->joinsBelow() && crop->joinsAbove());
        QCOMPARE(crop->y(), time->geometry().bottom() + 1);
        QCOMPARE(time->y() - compositing->geometry().bottom() - 1, 0);
        QCOMPARE(compositing->height() - compositing->bodyWidget()->geometry().bottom() - 1, InspectorSection::kGap);
        // Opening one parts the rack round it, and folding it closes it again.
        crop->setOpen(true);
        QVERIFY(!time->joinsBelow() && !crop->joinsAbove());
        crop->setOpen(false);
        QVERIFY(time->joinsBelow() && crop->joinsAbove());
        // The titles are engraved: bold capitals, tracked, named whole for
        // assistive technology.
        QCOMPARE(transform->titleLabel()->font().weight(), QFont::Bold);
        QCOMPARE(transform->titleLabel()->font().capitalization(), QFont::AllUppercase);
        QCOMPARE(transform->titleLabel()->accessibleName(), QStringLiteral("Transform"));
        QCOMPARE(compositing->title(), QStringLiteral("Composite"));
        QCOMPARE(transform->header()->height(), InspectorSection::kHeaderHeight);
        // A header takes focus from Tab, named with its fold; Space or
        // Return folds or opens it (Space no longer play there), and it is
        // ringed as a focused field is.
        {
            QWidget *header = time->header();
            QCOMPARE(header->focusPolicy(), Qt::TabFocus);
            QCOMPARE(header->accessibleName(), QStringLiteral("Time, collapsed"));
            inspector.activateWindow();
            QVERIFY(QTest::qWaitForWindowActive(&inspector));
            const QPoint ringAt(header->x() + 2, header->y() + header->height() / 2);
            const auto ringPixel = [time, ringAt] {
                const QImage image = time->grab().toImage();
                return image.pixelColor(ringAt * image.devicePixelRatio());
            };
            QVERIFY(!nearColor(ringPixel().rgb(), editorTheme().focusRing.rgb(), 8));
            header->setFocus(Qt::TabFocusReason);
            QVERIFY(header->hasFocus());
            QVERIFY2(nearColor(ringPixel().rgb(), editorTheme().focusRing.rgb(), 8), qPrintable(ringPixel().name()));
            QKeyEvent override(QEvent::ShortcutOverride, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
            override.ignore();
            QCoreApplication::sendEvent(header, &override);
            QVERIFY(override.isAccepted());
            QSignalSpy toggled(time, &InspectorSection::toggled);
            QTest::keyClick(header, Qt::Key_Space);
            QVERIFY(time->isOpen());
            QCOMPARE(toggled.size(), 1);
            QCOMPARE(header->accessibleName(), QStringLiteral("Time, expanded"));
            QTest::keyClick(header, Qt::Key_Return);
            QVERIFY(!time->isOpen());
            QCOMPARE(toggled.size(), 2);
            QCOMPARE(header->accessibleName(), QStringLiteral("Time, collapsed"));
            header->clearFocus();
        }

        // Lamps: dark at the defaults; a moved value lights its module's and
        // rests in its row's lane until the pointer brings ↺.
        auto *scaleRow = qobject_cast<InspectorRow *>(inspector.findChild<ScrubField *>(QStringLiteral("prop_scale"))->parentWidget());
        QVERIFY(scaleRow);
        QCOMPARE(transform->lamp()->state(), LampState::Off);
        QVERIFY(!scaleRow->lamp()->isVisibleTo(scaleRow));
        inspector.setState(withProps({{"scale", 1.12}, {"blend", "multiply"}}), {QStringLiteral("v")});
        QCOMPARE(transform->lamp()->state(), LampState::Changed);
        QCOMPARE(transform->lamp()->accessibleName(), QStringLiteral("Changed"));
        QCOMPARE(compositing->lamp()->state(), LampState::Changed);
        QCOMPARE(crop->lamp()->state(), LampState::Off);
        QCOMPARE(scaleRow->lamp()->state(), LampState::Changed);
        QCOMPARE(scaleRow->lamp()->diameter(), InspectorRow::kLampDiameter);
        QVERIFY(scaleRow->lamp()->isVisibleTo(scaleRow) && !scaleRow->resetButton()->isVisibleTo(scaleRow));
        {
            QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), QPointF(4, 4));
            QCoreApplication::sendEvent(scaleRow, &enter);
        }
        QVERIFY(!scaleRow->lamp()->isVisibleTo(scaleRow) && scaleRow->resetButton()->isVisibleTo(scaleRow));
        {
            QEvent leave(QEvent::Leave);
            QCoreApplication::sendEvent(scaleRow, &leave);
        }
        QVERIFY(scaleRow->lamp()->isVisibleTo(scaleRow));
        // The changed value reads in the accent; at its default, quiet.
        auto *scale = inspector.findChild<ScrubField *>(QStringLiteral("prop_scale"));
        QCOMPARE(scale->valueInk(), changedValueInk());
        QCOMPARE(inspector.findChild<ScrubField *>(QStringLiteral("prop_opacity"))->valueInk(), editorTheme().textSecondary);

        // The pop-up is clear over a well its row presses under it, and in
        // the accent once moved from its default.
        auto *blend = inspector.findChild<QComboBox *>(QStringLiteral("prop_blend"));
        QVERIFY(blend);
        auto *blendRow = qobject_cast<InspectorRow *>(blend->parentWidget());
        QVERIFY(blendRow);
        QCOMPARE(blendRow->wells(), (QList<QRect>{blend->geometry()}));
        QVERIFY(blend->property("changed").toBool());
        inspector.setState(withProps({{"scale", 1.12}}), {QStringLiteral("v")});
        QVERIFY(!blend->property("changed").toBool());

        // The timing on one glass: In, Out and Dur over cream figures that
        // still scrub and take typing, each named for what it is.
        QCOMPARE(inspector.m_timing->labels(), (QStringList{"In", "Out", "Dur"}));
        QCOMPARE(inspector.m_timingFields.size(), 3);
        const QStringList names{"Start", "End", "Duration"};
        for (int index = 0; index < 3; ++index) {
            ScrubField *field = inspector.m_timingFields.at(index);
            QVERIFY(field->isGlass());
            QCOMPARE(field->accessibleName(), names.at(index));
            QCOMPARE(field->parentWidget(), static_cast<QWidget *>(inspector.m_timing));
            QCOMPARE(field->valueInk(), editorTheme().glassText);
        }
        QCOMPARE(inspector.m_timingFields.at(0)->text(), QStringLiteral("00:00:06:00"));
        QVERIFY(inspector.m_timingFields.at(0)->showsWhole());
        inspector.m_timingFields.at(0)->commitText(QStringLiteral("+12"));
        QCOMPARE(sent.last().first().toObject().value("op").toString(), QStringLiteral("move"));
        QCOMPARE(sent.last().first().toObject().value("frames").toInt(), 12);

        // Fit and Fill: an exclusive bank, the matching key latched (none
        // after a scale that is neither); a press latches its key at once.
        auto *fit = inspector.findChild<KeyBank *>(QStringLiteral("fitFill"));
        QVERIFY(fit && fit->isExclusive());
        QCOMPARE(fit->count(), 2);
        QCOMPARE(fit->current(), -1);
        QCOMPARE(fit->key(0)->accessibleName(), QStringLiteral("Fit"));
        QCOMPARE(fit->key(1)->toolTip(), QStringLiteral("The frame filled, edges cropped"));
        QVERIFY(fit->key(1)->isCheckable());
        fit->key(1)->click();
        QCOMPARE(fit->current(), 1);
        QVERIFY(lastProps().value("scale").toDouble() > 3.0);
        inspector.setState(withProps({}), {QStringLiteral("v")});
        QCOMPARE(fit->current(), 0);
        // A press on the key already in changes nothing on the bank.
        fit->key(0)->click();
        QCOMPARE(fit->current(), 0);

        // Flip: H and V latch on their own.
        inspector.section(QStringLiteral("anchor"))->setOpen(true);
        auto *flip = inspector.findChild<KeyBank *>(QStringLiteral("flip"));
        QVERIFY(flip && !flip->isExclusive());
        QCOMPARE(flip->width(), InspectorRow::kValueField);
        QVERIFY(!flip->isLatched(0) && !flip->isLatched(1));
        flip->key(0)->click();
        QCOMPARE(lastProps().value("scale_x").toDouble(), -1.0);
        inspector.setState(withProps({{"scale_x", -1.0}}), {QStringLiteral("v")});
        QVERIFY(flip->isLatched(0) && !flip->isLatched(1));
        QCOMPARE(inspector.section(QStringLiteral("anchor"))->lamp()->state(), LampState::Changed);
        flip->key(0)->click();
        QVERIFY(lastProps().value("scale_x").isNull());

        // The knob beside Rotation: a drag up turns it clockwise, one edit
        // on release, the field following it; the wheel's run of steps is
        // one edit too; a double-click turns it back.
        auto *knob = inspector.findChild<RotationKnob *>(QStringLiteral("rotationKnob"));
        auto *rotation = inspector.findChild<ScrubField *>(QStringLiteral("prop_rotation"));
        QVERIFY(knob && rotation);
        QCOMPARE(knob->size(), QSize(RotationKnob::kSize, RotationKnob::kSize));
        QCOMPARE(knob->accessibleName(), QStringLiteral("Rotation"));
        const QPoint middle = knob->rect().center();
        const qsizetype before = sent.size();
        QTest::mousePress(knob, Qt::LeftButton, Qt::NoModifier, middle);
        QTest::mouseMove(knob, middle - QPoint(0, 30));
        QCOMPARE(sent.size(), before);
        QCOMPARE(rotation->value(), 30.0);
        QTest::mouseRelease(knob, Qt::LeftButton, Qt::NoModifier, middle - QPoint(0, 30));
        QCOMPARE(sent.size(), before + 1);
        QCOMPARE(lastProps().value("rotation").toDouble(), 30.0);
        inspector.setState(withProps({{"rotation", 30.0}}), {QStringLiteral("v")});
        QCOMPARE(knob->angle(), 30.0);
        for (int notch = 0; notch < 3; ++notch) {
            QWheelEvent wheel(QPointF(middle), knob->mapToGlobal(QPointF(middle)), QPoint(), QPoint(0, QWheelEvent::DefaultDeltasPerStep),
                              Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
            QCoreApplication::sendEvent(knob, &wheel);
        }
        QCOMPARE(knob->angle(), 33.0);
        QCOMPARE(sent.size(), before + 1);
        knob->flushWheel();
        QCOMPARE(sent.size(), before + 2);
        QCOMPARE(lastProps().value("rotation").toDouble(), 33.0);
        inspector.setState(withProps({{"rotation", 33.0}}), {QStringLiteral("v")});
        QTest::mouseDClick(knob, Qt::LeftButton, Qt::NoModifier, middle);
        QVERIFY(lastProps().contains("rotation") && lastProps().value("rotation").isNull());

        // Each effect's power key: held in with its lamp lit while on.
        inspector.setState(withProps({}, QJsonArray{QJsonObject{{"fx", "blur.gaussian"}, {"enabled", false}, {"params", QJsonObject{}}}}),
                           {QStringLiteral("v")});
        auto *power = inspector.findChild<PowerKey *>(QStringLiteral("effectEnabled"));
        QVERIFY(power);
        QVERIFY(!power->isChecked());
        QCOMPARE(power->lampState(), LampState::Off);
        QCOMPARE(power->accessibleName(), QStringLiteral("On"));
        power->click();
        QCOMPARE(sent.last().first().toObject().value("enabled").toBool(), true);
        inspector.setState(withProps({}, QJsonArray{QJsonObject{{"fx", "blur.gaussian"}, {"enabled", true}, {"params", QJsonObject{}}}}),
                           {QStringLiteral("v")});
        power = inspector.findChild<PowerKey *>(QStringLiteral("effectEnabled"));
        QVERIFY(power->isChecked());
        QCOMPARE(power->lampState(), LampState::Ok);
    }

    // The knob beside Rotation: a click turns nothing, a drag lands on the
    // field's tenths, a double-click puts every clip selected back at 0°,
    // and a run of wheel steps edits the clips it was turned for, though
    // the selection changes before its pause ends.
    void inspectorRotationKnobEditsTheSelection() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        const auto clip = [](const char *id, double start, double rotation) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", start}, {"duration", 2.0},
                               {"props", QJsonObject{{"rotation", rotation}}}};
        };
        const QJsonObject timeline{
            {"fps", 24}, {"width", 1080}, {"height", 1920}, {"properties", catalogProperties()},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("v1", 0.0, 30.0), clip("v2", 2.0, 10.0)}}}}}};
        InspectorPanel inspector;
        inspector.setEffectCatalog(QJsonArray{});
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        inspector.setState(timeline, {QStringLiteral("v1")});
        inspector.resize(300, 900);
        inspector.show();
        QVERIFY(QTest::qWaitForWindowExposed(&inspector));
        const auto shownKnob = [&inspector]() -> RotationKnob * {
            for (RotationKnob *knob : inspector.findChildren<RotationKnob *>())
                if (knob->isVisible()) return knob;
            return nullptr;
        };
        RotationKnob *knob = shownKnob();
        QVERIFY(knob);
        const QPointF middle = QRectF(knob->rect()).center();
        const auto mouse = [knob](QEvent::Type type, const QPointF &at) {
            QMouseEvent event(type, at, knob->mapToGlobal(at), Qt::LeftButton,
                              type == QEvent::MouseButtonRelease ? Qt::NoButton : Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(knob, &event);
        };

        // A click that wanders a point or two turns nothing.
        mouse(QEvent::MouseButtonPress, middle);
        mouse(QEvent::MouseMove, middle - QPointF(0, 2));
        mouse(QEvent::MouseButtonRelease, middle - QPointF(0, 2));
        QVERIFY(sent.isEmpty());
        QCOMPARE(knob->angle(), 30.0);
        // A drag of 12.46 points lands on the field's tenth of a degree.
        mouse(QEvent::MouseButtonPress, middle);
        mouse(QEvent::MouseMove, middle - QPointF(0, 12.46));
        mouse(QEvent::MouseButtonRelease, middle - QPointF(0, 12.46));
        QCOMPARE(sent.size(), 1);
        QCOMPARE(sent.last().first().toObject().value("props").toObject().value("rotation").toDouble(), 42.5);

        // A double-click on two clips at 30° and 10°: both at 0°, not the
        // second shifted by the first's turn.
        sent.clear();
        inspector.setState(timeline, {QStringLiteral("v1"), QStringLiteral("v2")});
        knob = shownKnob();
        QVERIFY(knob);
        QTest::mouseDClick(knob, Qt::LeftButton, Qt::NoModifier, knob->rect().center());
        QCOMPARE(sent.size(), 1);
        QStringList reset;
        for (const QJsonValue &op : sent.last()) {
            QVERIFY(op.toObject().value("props").toObject().value("rotation").isNull());
            reset << op.toObject().value("clip").toString();
        }
        QCOMPARE(reset, (QStringList{"v1", "v2"}));

        // Two wheel steps on the first clip, then the second selected
        // before the pause ends: the edit goes to the first, then and once.
        sent.clear();
        inspector.setState(timeline, {QStringLiteral("v1")});
        knob = shownKnob();
        QVERIFY(knob);
        for (int notch = 0; notch < 2; ++notch) {
            QWheelEvent wheel(middle, knob->mapToGlobal(middle), QPoint(), QPoint(0, QWheelEvent::DefaultDeltasPerStep),
                              Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
            QCoreApplication::sendEvent(knob, &wheel);
        }
        QVERIFY(sent.isEmpty());
        inspector.setState(timeline, {QStringLiteral("v2")});
        QCOMPARE(sent.size(), 1);
        const QJsonObject op = sent.last().first().toObject();
        QCOMPARE(op.value("clip").toString(), QStringLiteral("v1"));
        QCOMPARE(op.value("props").toObject().value("rotation").toDouble(), 32.0);
        QTest::qWait(RotationKnob::kWheelRunMs + 100);
        QCOMPARE(sent.size(), 1);
    }

    // The parts are painted from the hardware tokens in both appearances,
    // and their text reads at 4.5:1 on what it is printed on.
    void inspectorHardwareColours() {
        {
            const EditorTheme &theme = editorTheme();
            const QString name = QStringLiteral("dark");
            // Labels on the plate, values in the well, legends on the keys.
            for (const QColor &surface : {theme.modulePlate, theme.wellFill})
                QVERIFY2(contrastRatio(theme.textSecondary, surface) >= 4.5, qPrintable(name));
            QVERIFY2(contrastRatio(theme.text, theme.wellFill) >= 4.5, qPrintable(name));
            QVERIFY2(contrastRatio(changedValueInk(), theme.wellFill) >= 4.5, qPrintable(name));
            QVERIFY2(contrastRatio(theme.accentText, theme.wellFill) >= 4.5, qPrintable(name));
            for (const QColor &key : {theme.keyTop, theme.keyBottom, theme.keyLatched})
                QVERIFY2(contrastRatio(theme.engravedInk, key) >= 4.5, qPrintable(name));
            // A value's well.
            ScrubField field;
            field.resize(InspectorRow::kValueField, ScrubField::kHeight);
            QImage image(field.size(), QImage::Format_ARGB32_Premultiplied);
            image.fill(theme.modulePlate);
            field.render(&image, QPoint(), QRegion(), QWidget::DrawChildren);
            QCOMPARE(image.pixelColor(field.width() / 2, field.height() / 2).rgb(), theme.wellFill.rgb());
            // A latched key's amber bar, lit at its top; a raised key has none.
            KeyBank bank({{QStringLiteral("Fit"), QString(), QStringLiteral("a")}, {QStringLiteral("Fill"), QString(), QStringLiteral("b")}}, true);
            bank.resize(96, ScrubField::kHeight);
            bank.setCurrent(0);
            QCoreApplication::processEvents();
            QImage keys(bank.size(), QImage::Format_ARGB32_Premultiplied);
            keys.fill(theme.modulePlate);
            bank.render(&keys, QPoint(), QRegion(), QWidget::DrawWindowBackground | QWidget::DrawChildren);
            const QRect lit = bank.key(0)->geometry();
            const QRect raised = bank.key(1)->geometry();
            QCOMPARE(keys.pixelColor(lit.center().x(), lit.top() + 3).rgb(), theme.keyLed.rgb());
            QVERIFY(keys.pixelColor(raised.center().x(), raised.top() + 3).rgb() != theme.keyLed.rgb());
            // The bank's well shows between and round the keys.
            QCOMPARE(keys.pixelColor(lit.right() + 1, bank.height() / 2).rgb(), theme.bankWell.rgb());
            // The glass is dark in both, its figures cream.
            GlassReadout glass;
            glass.resize(240, 40);
            QImage readout(glass.size(), QImage::Format_ARGB32_Premultiplied);
            readout.fill(theme.panel);
            glass.render(&readout);
            QVERIFY(readout.pixelColor(120, 30).lightnessF() < 0.2);
        }
    }

    // Every part paints inside the rectangle it is given, its drop and lip
    // and a lamp's glow included, in both appearances and at 1x, 1.25x and
    // 2x; and its straight edges land on whole device pixels.
    void hardwarePartsStayInTheirBounds() {
        const QRectF rect(10, 10, 100, 30);
        const QRectF knobBox(10, 10, 32, 32);
        const QList<std::pair<const char *, std::function<void(QPainter &)>>> parts = {
            {"key", [&](QPainter &p) { paintRaisedKey(p, rect, false, false, false); }},
            {"hovered key", [&](QPainter &p) { paintRaisedKey(p, rect, false, true, false); }},
            {"latched lit key", [&](QPainter &p) { paintRaisedKey(p, rect, true, true, true); }},
            {"bank", [&](QPainter &p) { paintKeyBank(p, rect); }},
            {"well", [&](QPainter &p) { paintWell(p, rect); }},
            {"module", [&](QPainter &p) { paintModulePlate(p, rect); }},
            {"glass", [&](QPainter &p) { paintGlass(p, rect); }},
            {"engraved", [&](QPainter &p) { drawEngraved(p, rect, Qt::AlignCenter, QStringLiteral("Speed")); }},
            {"engraved hangul", [&](QPainter &p) { drawEngraved(p, rect, Qt::AlignCenter, QStringLiteral("속도")); }},
            {"vu", [&](QPainter &p) { paintVuMeter(p, rect, 0.9); }},
        };
        {
            for (const qreal ratio : {1.0, 1.25, 2.0}) {
                const QByteArray where = QByteArray::number(ratio) + (" dark");
                for (const auto &[name, paint] : parts) {
                    const QImage image = paintedAt(ratio, QSize(120, 50), paint);
                    QVERIFY2(!paintsOutside(image, rect), QByteArray(name) + " at " + where);
                    bool painted = false;
                    for (int y = 0; y < image.height() && !painted; ++y)
                        for (int x = 0; x < image.width() && !painted; ++x) painted = qAlpha(image.pixel(x, y)) != 0;
                    QVERIFY2(painted, QByteArray(name) + " at " + where);
                }
                for (const double turn : {-135.0, 0.0, 135.0}) {
                    const QImage knob = paintedAt(ratio, QSize(60, 60), [&](QPainter &p) { paintKnob(p, knobBox, turn); });
                    QVERIFY2(!paintsOutside(knob, knobBox), "knob at " + where);
                }
                for (const LampState state : {LampState::Off, LampState::Changed, LampState::Kerf, LampState::Ok}) {
                    const QImage lamp = paintedAt(ratio, QSize(60, 40), [state](QPainter &p) { paintLamp(p, {30, 20}, 6, state); });
                    QVERIFY2(!paintsOutside(lamp, QRectF(24, 14, 12, 12)), "lamp at " + where);
                    QVERIFY2(qAlpha(lamp.pixel(int(30 * ratio), int(20 * ratio))) == 255, "lamp at " + where);
                }
                // A module's top and left edges are whole rows and columns of
                // its edge colour, with nothing above or beside them.
                const QImage plate = paintedAt(ratio, QSize(120, 50), [&](QPainter &p) { paintModulePlate(p, rect); });
                const int edge = int(std::round(10 * ratio));
                const int middle = int(60 * ratio);
                const QColor edgeColor = editorTheme().moduleEdge;
                QCOMPARE(qAlpha(plate.pixel(middle, edge - 1)), 0);
                QCOMPARE(QColor::fromRgba(plate.pixel(middle, edge)), edgeColor);
                QCOMPARE(qAlpha(plate.pixel(edge - 1, int(25 * ratio))), 0);
                QCOMPARE(QColor::fromRgba(plate.pixel(edge, int(25 * ratio))), edgeColor);
                // Its 1px edge is one device pixel at 1.25x, not a blur
                // across two; under it, flat, the plate itself (no line of
                // light any more).
                const int line = int(std::round(ratio));
                QCOMPARE(QColor::fromRgba(plate.pixel(middle, edge + line - 1)), edgeColor);
                QCOMPARE(QColor::fromRgba(plate.pixel(middle, edge + line)), editorTheme().modulePlate);
                QCOMPARE(QColor::fromRgba(plate.pixel(middle, edge + 2 * line)), editorTheme().modulePlate);
            }
        }
    }

    // Flat keys: a latched key is the on fill, lighter than a raised one's
    // `raised`, and flat across its face; under the pointer a key brightens;
    // lit, its accent bar shows at its top.
    void hardwareKeyStates() {
        const QRectF key(10, 10, 40, 24);
        {
            const EditorTheme &t = editorTheme();
            const auto keyImage = [&](bool latched, bool hovered, bool lit) {
                return paintedAt(2, QSize(60, 44), [&](QPainter &p) { paintRaisedKey(p, key, latched, hovered, lit); });
            };
            const QImage raised = keyImage(false, false, false);
            const QImage latched = keyImage(true, false, false);
            QVERIFY(raised != latched);
            QVERIFY(lightnessAt(latched, {30, 24}) > lightnessAt(raised, {30, 24}));
            // No shadow or bevel: the face is one fill, top to foot.
            QCOMPARE(lightnessAt(latched, {30, 14}), lightnessAt(latched, {30, 30}));
            QCOMPARE(lightnessAt(raised, {30, 14}), lightnessAt(raised, {30, 30}));
            QVERIFY(keyImage(false, true, false) != raised);
            const QImage lit = keyImage(false, false, true);
            const QColor led = QColor::fromRgba(lit.pixel(30 * 2, 13 * 2 + 1));
            QVERIFY2(std::abs(led.hue() - t.keyLed.hue()) < 12 && led.saturation() > 120, qPrintable(led.name()));
            QVERIFY(lit != raised);
            // Unlit, the same spot is the key's own face.
            QVERIFY(QColor::fromRgba(raised.pixel(30 * 2, 13 * 2 + 1)).saturation() < 120);
        }
    }

    // The knob's pointer turns with its angle, from straight up clockwise.
    void hardwareKnobPointerFollowsTheTurn() {
        const QRectF box(0, 0, 32, 32);
        const QPointF centre = box.center();
        const auto pointerAt = [&](double degrees) {
            const qreal radians = qDegreesToRadians(degrees);
            return centre + QPointF(std::sin(radians), -std::cos(radians)) * 6;
        };
        for (const qreal ratio : {1.0, 2.0}) {
            for (const double turn : {0.0, 90.0, -135.0, 135.0}) {
                const QImage knob = paintedAt(ratio, QSize(32, 32), [&](QPainter &p) { paintKnob(p, box, turn); });
                const int pointer = lightnessAt(knob, pointerAt(turn));
                QVERIFY2(pointer > 160, qPrintable(QStringLiteral("%1° at %2x: %3").arg(turn).arg(ratio).arg(pointer)));
                // A quarter turn away is the knob's dark face.
                QVERIFY(lightnessAt(knob, pointerAt(turn + 90)) < 100);
                QVERIFY(lightnessAt(knob, pointerAt(turn + 180)) < 100);
            }
        }
    }

    // Engraved labels are tracked capitals, except Hangul and other CJK
    // text, which has no capitals and loses its shapes when spaced: it is
    // untracked, as written, and a pixel larger. The ink is painted over its
    // catch of light.
    void hardwareEngravedLabels() {
        QFont base;
        base.setPixelSize(13);
        const QFont latin = engravedFont(base, QStringLiteral("Speed"));
        QCOMPARE(latin.pixelSize(), 10);
        QCOMPARE(latin.weight(), QFont::Bold);
        QCOMPARE(latin.letterSpacingType(), QFont::AbsoluteSpacing);
        QVERIFY(std::abs(latin.letterSpacing() - 1.3) < 0.01); // Kept in 64ths of a pixel.
        QCOMPARE(latin.capitalization(), QFont::AllUppercase);
        QCOMPARE(latin.family(), base.family());
        for (const QString &text : {QStringLiteral("속도"), QStringLiteral("速度"), QStringLiteral("スピード"),
                                    QStringLiteral("Speed 속도")}) {
            const QFont cjk = engravedFont(base, text);
            QCOMPARE(cjk.pixelSize(), 11);
            QCOMPARE(cjk.weight(), QFont::Bold);
            QCOMPARE(cjk.letterSpacing(), 0.0);
            QCOMPARE(cjk.capitalization(), QFont::MixedCase);
        }
        {
            TextRecorder recorder;
            QPainter painter(&recorder);
            drawEngraved(painter, QRectF(0, 0, 200, 20), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("Speed"));
            painter.end();
            QVERIFY(recorder.runs.size() >= 2);
            QCOMPARE(recorder.runs.first().color, editorTheme().engravedHighlight);
            QCOMPARE(recorder.runs.last().color, editorTheme().engravedInk);
        }
    }

    // Pictures of every hardware part on the faceplate, to look at: run with
    // KERF_HARDWARE_GALLERY set to a folder. Each appearance is drawn at 1x,
    // 1.25x and 2x.
    void hardwareGallery() {
        const QString folder = qEnvironmentVariable("KERF_HARDWARE_GALLERY");
        if (folder.isEmpty()) QSKIP("Set KERF_HARDWARE_GALLERY to draw the hardware parts.");
        QDir().mkpath(folder);
        {
            const EditorTheme &t = editorTheme();
            for (const qreal ratio : {1.0, 1.25, 2.0}) {
                const QImage image = paintedAt(ratio, QSize(560, 200), [&](QPainter &p) {
                    QLinearGradient plate(0, 0, 0, 200);
                    plate.setColorAt(0, t.faceplateTop);
                    plate.setColorAt(1, t.faceplateBottom);
                    p.fillRect(QRectF(0, 0, 560, 200), plate);
                    p.fillRect(QRectF(0, 0, 560, 1), t.faceplateHighlight);
                    p.fillRect(QRectF(0, 199, 560, 1), t.faceplateEdge);
                    // A bank of three keys: raised, hovered, latched and lit.
                    paintKeyBank(p, QRectF(12, 12, 4 + 3 * 30 + 2 * 2, 30));
                    for (int key = 0; key < 3; ++key)
                        paintRaisedKey(p, QRectF(14 + key * 32, 14, 30, 26), key == 2, key == 1, key == 2);
                    paintWell(p, QRectF(130, 14, 70, 24));
                    p.setFont(monoFont(12));
                    p.setPen(t.accentText);
                    p.drawText(QRectF(130, 14, 66, 23), Qt::AlignRight | Qt::AlignVCenter, QStringLiteral("1.25×"));
                    paintGlass(p, QRectF(214, 10, 150, 32));
                    p.setFont(monoFont(16, QFont::Medium));
                    p.setPen(t.glassText);
                    p.drawText(QRectF(240, 10, 120, 31), Qt::AlignRight | Qt::AlignVCenter, QStringLiteral("00:00:12:04"));
                    QFont label = monoFont(9, QFont::Bold);
                    label.setLetterSpacing(QFont::AbsoluteSpacing, 1.2);
                    p.setFont(label);
                    p.setPen(t.glassLabel);
                    p.drawText(QRectF(222, 10, 30, 31), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("TC"));
                    // A module with engraved labels, lamps and a knob.
                    paintModulePlate(p, QRectF(12, 56, 260, 130));
                    drawEngraved(p, QRectF(26, 64, 120, 18), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("Speed"));
                    drawEngraved(p, QRectF(150, 64, 110, 18), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("속도 조절"));
                    int lamp = 0;
                    for (const LampState state : {LampState::Off, LampState::Changed, LampState::Kerf, LampState::Ok})
                        paintLamp(p, QPointF(32 + 20 * lamp++, 100), 6, state);
                    for (int knob = 0; knob < 3; ++knob)
                        paintKnob(p, QRectF(26 + knob * 44, 120, 32, 32), -135 + knob * 135);
                    paintKnob(p, QRectF(170, 104, 64, 64), 30);
                    paintVuMeter(p, QRectF(290, 56, 150, 90), 0.62);
                    paintVuMeter(p, QRectF(456, 56, 90, 54), 0.9);
                });
                const QString look = QStringLiteral("dark");
                QVERIFY(image.save(QDir(folder).filePath(QStringLiteral("%1-%2x.png").arg(look).arg(ratio))));
            }
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::InspectorTest)

#include "inspector_test.moc"
