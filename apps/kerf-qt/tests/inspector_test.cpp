// Tests for the Inspector and its controls.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "bridge.h"
#include "components.h"
#include "deliver_pane.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "inspector_column.h"
#include "wiring/inspector_wiring.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
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
#include <QWheelEvent>
#include <QWidget>


#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

namespace {

// The property row a control sits in.
InspectorRow *rowOf(QWidget *widget) {
    for (QWidget *at = widget; at; at = at->parentWidget())
        if (auto *row = qobject_cast<InspectorRow *>(at)) return row;
    return nullptr;
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

    // The slider is the kit's .sl in a 24-point row: a 2pt track in
    // `track` at the middle, the fill in `track2` from its origin to the
    // knob, a 10pt ink knob; a bipolar slider has the 1 x 6 `track2` tick at
    // its middle (drawn under the knob, never on it), a one-sided one none.
    void inspectorSliderDefaultShowsAroundTheKnob() {
        InspectorSlider slider;
        slider.resize(100, ScrubField::kHeight);
        slider.setRange(-3.0, 3.0);
        slider.setOrigin(0.0);
        slider.setDetent(0.0);
        QVERIFY(slider.isBipolar());
        const QRectF track = slider.trackRect();
        QCOMPARE(track.height(), 2.0);
        QCOMPARE(track.center().y(), ScrubField::kHeight / 2.0);
        QCOMPARE(track.left(), InspectorSlider::kKnobRadius);
        QCOMPARE(slider.fillInk(), tokens::color(tokens::track2));
        for (const double value : {0.0, 1.5}) {
            slider.setValue(value);
            const QRect tick = slider.detentTick();
            QCOMPARE(tick.size(), QSize(1, 6));
            QVERIFY(std::abs(tick.left() - (track.left() + track.width() / 2)) <= 1.0);
            QImage image(slider.size() * 2, QImage::Format_ARGB32_Premultiplied);
            image.setDevicePixelRatio(2);
            image.fill(tokens::color(tokens::chrome));
            slider.render(&image);
            const QPointF knob = slider.knobCenter();
            // The knob's middle is the ink, whatever lies under it.
            QCOMPARE(image.pixelColor((knob * 2).toPoint()).rgb(), tokens::color(tokens::ink).rgb());
            // The track away from the knob and the fill.
            const QPointF bare(track.left() + 2, track.center().y());
            QVERIFY(nearColor(image.pixelColor((bare * 2).toPoint()).rgb(), tokens::color(tokens::track).rgb(), 24));
            if (value > 0) {
                const QRectF fill = slider.fillRect();
                QVERIFY(std::abs(fill.left() - (track.left() + track.width() / 2)) < 0.5);
                QVERIFY(std::abs(fill.right() - knob.x()) < 0.5);
            } else {
                QVERIFY(slider.fillRect().isEmpty());
            }
        }
        // One-sided (an opacity): no tick, filled from the left.
        slider.setRange(0.0, 100.0);
        slider.setOrigin(0.0);
        slider.setDetent(100.0);
        slider.setValue(42.0);
        QVERIFY(!slider.isBipolar());
        QVERIFY(slider.detentTick().isEmpty());
        QCOMPARE(slider.fillRect().left(), slider.trackRect().left());
        // Disabled (an animated value outside its clip): dis.
        slider.setEnabled(false);
        QCOMPARE(slider.fillInk(), tokens::color(tokens::dis));
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
        QVERIFY(!box->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"))->isChecked());
        // The secondary ink reads at 4.5:1 on the column.
        QVERIFY(contrastRatio(tokens::color(tokens::ink2), tokens::color(tokens::chrome)) >= 4.5);
        const auto fields = box->findChildren<ScrubField *>();
        QVERIFY(!fields.isEmpty());
        for (ScrubField *field : fields) {
            QVERIFY(field->isDimmed() && field->isEnabled());
            QCOMPARE(field->valueInk(), tokens::color(tokens::ink2));
        }
        const auto sliders = box->findChildren<InspectorSlider *>();
        QVERIFY(!sliders.isEmpty());
        for (InspectorSlider *slider : sliders) {
            QVERIFY(slider->isDimmed() && slider->isEnabled());
            QCOMPARE(slider->fillInk(), tokens::color(tokens::ink3));
        }
        for (QLabel *hex : box->findChildren<QLabel *>(QStringLiteral("colorHex")))
            QVERIFY(hex->property("off").toBool());
        QVERIFY(box->findChild<QLabel *>(QStringLiteral("effectName"))->property("off").toBool());
    }

    // Each effect is a row with its enable box, its name and More (Reset,
    // Ask Kerf, Remove), no ✕; its parameters are rows with units, five
    // before "N more", grouped under quiet sub-headers when the catalog
    // groups them.
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
        // Open (the design's first group), counting its effects; its
        // tooltip names every one, one switched off marked so.
        QVERIFY(effects->isOpen());
        QCOMPARE(effects->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("2"));
        QCOMPARE(effects->summary(), QStringLiteral("Edge Glow, Invert (off)"));
        // Its box: every effect on or off (one is on).
        QCOMPARE(effects->enable(), InspectorSection::Enable::On);
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
        }
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 2);
        auto *glow = boxes.at(0)->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"));
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
        QVERIFY(!boxes.at(1)->findChild<InspectorSwitch *>(QStringLiteral("effectEnabled"))->isChecked());
        QVERIFY(boxes.at(1)->findChild<QLabel *>(QStringLiteral("effectName"))->property("off").toBool());
        // Folded at first: its rows hidden under its own row.
        QVERIFY(boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        // No ✕ anywhere.
        for (QAbstractButton *button : inspector->findChildren<QAbstractButton *>()) {
            QVERIFY(button->text() != QStringLiteral("✕"));
            QVERIFY(button->objectName() != QStringLiteral("effectRemove"));
        }
        // Opening it shows its rows and hides the line; it stays open for the effect.
        boxes.at(0)->findChild<GlyphButton *>(QStringLiteral("effectChevron"))->click();
        QVERIFY(!boxes.at(0)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
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
        auto *tintRow = rowOf(grouped->findChild<ScrubField *>(QStringLiteral("effectParam_tint")));
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
        QCOMPARE(more->font().pixelSize(), 11);
        QWidget *moreRows = long7->findChild<QWidget *>(QStringLiteral("effectMoreRows"));
        QVERIFY(moreRows->isHidden());
        QVERIFY(moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p5"))));
        QVERIFY(!moreRows->isAncestorOf(long7->findChild<ScrubField *>(QStringLiteral("effectParam_p4"))));
        more->click();
        QVERIFY(!moreRows->isHidden());
        QVERIFY(QSettings().value(QStringLiteral("editor/inspector/fx/test.seven/more")).toBool());
    }

    // A light's parameters come from the catalog, as every generator's do: what softens its shadow
    // among them, with what the catalog says of it.
    void aSoftShadowsSizeIsInTheInspector() {
        // The catalog itself, as the bridge hands it over.
        char *all = kerf_fx_catalog_all();
        QVERIFY(all);
        const QJsonArray entries = QJsonDocument::fromJson(all).array();
        kerf_bridge_string_free(all);
        QJsonArray light;
        for (const QJsonValue &entry : entries)
            if (entry.toObject().value("id") == QLatin1String("light")) light = entry.toObject().value("params").toArray();
        const auto defOf = [&light](const char *name) {
            for (const QJsonValue &def : light)
                if (def.toObject().value("name") == QLatin1String(name)) return def.toObject();
            return QJsonObject{};
        };
        QCOMPARE(defOf("size").value("type").toString(), QStringLiteral("number"));
        QVERIFY(defOf("size").value("doc").toString().contains(QStringLiteral("Softens shadows")));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProperties()}, {"generators", QJsonObject{{"light", light}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "sun"}, {"kind", "generator"}, {"fx", "light"}, {"timelineStart", 0.0}, {"duration", 2.0},
                            {"props", QJsonObject{{"kind", "directional"}, {"shadow", 1.0}, {"size", 5.0}}}}}}}}}}}});
        window.selectClip(QStringLiteral("sun"));
        InspectorPanel *inspector = window.m_inspector;
        auto *size = inspector->findChild<ScrubField *>(QStringLiteral("prop_size"));
        auto *shadow = inspector->findChild<ScrubField *>(QStringLiteral("prop_shadow"));
        QVERIFY(size && shadow);
        QCOMPARE(size->text(), QStringLiteral("5.0"));
        // An edit of it goes to the document as any other does.
        size->commitText(QStringLiteral("8"));
        const QJsonArray ops = takeOps(window);
        QVERIFY(!ops.isEmpty());
        QCOMPARE(ops.last().toObject().value("props").toObject().value("size").toDouble(), 8.0);
    }

    // A clip's rows in depth come from the catalog as the bridge hands it over: a switch to go into
    // depth, the rest once it is on, a bevel once there is a solid to cut, and each edit goes to the
    // document. A camera and a light have a `z` of their own; what has no picture to place has none.
    void aClipInDepthIsInTheInspector() {
        char *properties = kerf_clip_properties();
        QVERIFY(properties);
        const QJsonObject catalogProps = QJsonDocument::fromJson(properties).object();
        kerf_bridge_string_free(properties);
        char *all = kerf_fx_catalog_all();
        QVERIFY(all);
        const QJsonArray entries = QJsonDocument::fromJson(all).array();
        kerf_bridge_string_free(all);
        QJsonObject generators;
        for (const QJsonValue &entry : entries)
            if (entry.toObject().value("kind") == QLatin1String("generator")) generators.insert(entry.toObject().value("id").toString(), entry.toObject().value("params"));
        for (const char *generator : {"solid", "camera", "light", "null", "adjustment"}) QVERIFY(generators.contains(QLatin1String(generator)));

        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        int revision = 5;
        const auto show = [&](const QJsonObject &props) {
            const auto generator = [](const char *id, const QJsonObject &own = {}) {
                return QJsonObject{{"id", id}, {"kind", "generator"}, {"fx", id}, {"timelineStart", 0.0}, {"duration", 2.0}, {"props", own}};
            };
            window.applySnapshot(QJsonObject{{"revision", ++revision}, {"duration", 4}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1920}, {"height", 1080}, {"properties", catalogProps}, {"generators", generators},
                {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    generator("solid", props), generator("camera", {{"z", -0.5}}), generator("light", {{"z", 0.25}}), generator("null"), generator("adjustment")}}}}}}}});
        };
        InspectorPanel *inspector = window.m_inspector;
        const auto field = [inspector](const char *name) { return inspector->findChild<ScrubField *>(QString::fromLatin1(name)); };
        const auto hidden = [&field](std::initializer_list<const char *> names) {
            return std::all_of(names.begin(), names.end(), [&field](const char *name) { return field(name) && rowOf(field(name))->isHidden(); });
        };
        const auto shown = [&field](std::initializer_list<const char *> names) {
            return std::all_of(names.begin(), names.end(), [&field](const char *name) { return field(name) && !rowOf(field(name))->isHidden(); });
        };

        // Flat: only the switch, whatever is left of the values it would use, and they say nothing.
        const auto profile = [inspector] { return inspector->findChild<QComboBox *>(QStringLiteral("prop_bevel_profile")); };
        const auto profileShows = [&profile] { return profile() && !rowOf(profile())->isHidden(); };
        show({{"z", 0.25}});
        window.selectClip(QStringLiteral("solid"));
        auto *threeD = inspector->findChild<InspectorSwitch *>(QStringLiteral("prop_three_d"));
        QVERIFY(threeD);
        QVERIFY(!threeD->isChecked());
        QVERIFY(!threeD->isHidden());
        QVERIFY(hidden({"prop_z", "prop_rotate_x", "prop_rotate_y", "prop_depth", "prop_bevel"}));
        InspectorSection *transform = inspector->section(QStringLiteral("transform"));
        QVERIFY(transform->summary().isEmpty());
        QVERIFY(!transform->isChanged());
        threeD->click();
        QJsonArray ops = takeOps(window);
        QVERIFY(!ops.isEmpty());
        QCOMPARE(ops.last().toObject().value("clip").toString(), QStringLiteral("solid"));
        QCOMPARE(ops.last().toObject().value("props").toObject().value("three_d").toBool(), true);

        // In depth: its values, in the frame's pixels and degrees, with no bevel until it is a solid.
        show({{"three_d", true}, {"z", 0.25}, {"rotate_x", 15.0}, {"rotate_y", -30.0}});
        QVERIFY(threeD->isChecked());
        QVERIFY(shown({"prop_z", "prop_rotate_x", "prop_rotate_y", "prop_depth"}));
        QVERIFY(hidden({"prop_bevel"}));
        QVERIFY(profile() && !profileShows());
        QCOMPARE(field("prop_z")->text(), QStringLiteral("270"));
        QCOMPARE(field("prop_z")->format().unit, QStringLiteral("px"));
        QCOMPARE(field("prop_rotate_x")->text(), QStringLiteral("15"));
        QCOMPARE(field("prop_rotate_x")->format().unit, QStringLiteral("°"));
        QCOMPARE(field("prop_rotate_x")->format().step, 1.0);
        QCOMPARE(field("prop_rotate_y")->text(), QStringLiteral("−30"));
        QCOMPARE(rowOf(field("prop_rotate_x"))->label()->fullText(), QStringLiteral("Rotate X"));
        QCOMPARE(rowOf(threeD)->label()->fullText(), QStringLiteral("3D"));
        QVERIFY(transform->summary().contains(QStringLiteral("3D")));
        QVERIFY(transform->summary().contains(QStringLiteral("Z 270 px")));
        QVERIFY(transform->isChanged());
        const auto committed = [&](const char *name, const char *text, const char *prop) {
            field(name)->commitText(QString::fromLatin1(text));
            const QJsonArray sent = takeOps(window);
            return sent.isEmpty() ? std::nan("") : sent.last().toObject().value("props").toObject().value(QString::fromLatin1(prop)).toDouble();
        };
        QCOMPARE(committed("prop_z", "540", "z"), 0.5);
        QCOMPARE(committed("prop_rotate_x", "45", "rotate_x"), 45.0);
        QCOMPARE(committed("prop_rotate_y", "-90", "rotate_y"), -90.0);
        QCOMPARE(committed("prop_depth", "108", "depth"), 0.1);

        // A solid has a bevel, and one that becomes solid as it goes has it too.
        show({{"three_d", true}, {"depth", 0.1}, {"bevel", 0.025}});
        QVERIFY(shown({"prop_depth", "prop_bevel"}));
        // Its profile waits with it, flat until it is said, and goes to the document by name.
        QVERIFY(profileShows());
        QCOMPARE(profile()->currentData().toString(), QStringLiteral("flat"));
        takeOps(window);
        profile()->setCurrentIndex(profile()->findData(QStringLiteral("round")));
        Q_EMIT profile()->activated(profile()->currentIndex());
        QCOMPARE(takeOps(window).last().toObject().value("props").toObject().value("bevel_profile").toString(), QStringLiteral("round"));
        QCOMPARE(field("prop_depth")->text(), QStringLiteral("108"));
        QCOMPARE(field("prop_bevel")->text(), QStringLiteral("27"));
        QCOMPARE(committed("prop_bevel", "54", "bevel"), 0.05);
        show({{"three_d", true}, {"depth", QJsonArray{QJsonObject{{"at", 0.0}, {"value", 0.0}, {"interp", "linear"}},
                                                     QJsonObject{{"at", 1.0}, {"value", 0.1}, {"interp", "linear"}}}}});
        QVERIFY(shown({"prop_bevel"}));

        // Out of depth again they go, though the document keeps them.
        show({{"depth", 0.1}, {"bevel", 0.025}});
        QVERIFY(hidden({"prop_z", "prop_depth", "prop_bevel"}));
        QVERIFY(transform->summary().isEmpty());

        // A camera and a light stand at their `z`, with nothing to switch on.
        for (const char *placed : {"camera", "light"}) {
            window.selectClip(QString::fromLatin1(placed));
            QVERIFY(shown({"prop_z"}));
            QVERIFY(!inspector->findChild<InspectorSwitch *>(QStringLiteral("prop_three_d")));
            QVERIFY(!field("prop_rotate_x") && !field("prop_depth") && !field("prop_bevel"));
        }
        QCOMPARE(field("prop_z")->text(), QStringLiteral("270"));
        QCOMPARE(committed("prop_z", "-540", "z"), -0.5);
        // A null and an adjustment layer have no picture to place.
        for (const char *unplaced : {"null", "adjustment"}) {
            window.selectClip(QString::fromLatin1(unplaced));
            QVERIFY(field("prop_x"));
            QVERIFY(!field("prop_z") && !inspector->findChild<InspectorSwitch *>(QStringLiteral("prop_three_d")));
        }
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
        // Animated: its value at the playhead (0.25 of 1080), editable (the
        // edit keys the playhead), its keyframe button lit on the key there.
        auto *xRow = rowOf(x);
        QVERIFY(xRow);
        QVERIFY(!x->isReadOnly());
        QCOMPARE(x->text(), QStringLiteral("270"));
        QVERIFY(xRow->isAnimated());
        QVERIFY(xRow->keyframeButton()->isAnimated() && xRow->keyframeButton()->has());
        inspector->setPlayhead(0.5);
        QCOMPARE(x->text(), QStringLiteral("675"));
        QVERIFY(!xRow->keyframeButton()->has());
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
        auto *fileLine = inspector->findChild<QLineEdit *>(QStringLiteral("effectParam_lut"));
        QVERIFY(fileLine);
        QVERIFY(!fileLine->parentWidget()->findChild<QPushButton *>());
        auto *fileChoose = fileLine->findChild<QAction *>(QStringLiteral("chooseFile"));
        QVERIFY(fileChoose && !fileChoose->icon().isNull());
        QCOMPARE(fileChoose->toolTip(), QStringLiteral("Choose a file"));
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
        window.selectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QTRY_VERIFY(window.m_timeline->hasFocus());
        window.m_shell->setSheet(StageShell::Sheet::Inspector);
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
        auto *row = rowOf(x);
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
        // Values are mono 12 in the ink, default or not; the unit is UI 11
        // ink3 in its 14-point slot, the axis label too.
        snapshot(18, QJsonObject{{"x", 0.1}});
        ScrubField *y = inspector->findChild<ScrubField *>(QStringLiteral("prop_y"));
        y->resize(78, ScrubField::kHeight);
        x->resize(78, ScrubField::kHeight);
        TextRecorder quiet;
        y->render(&quiet);
        QCOMPARE(quiet.colorOf(QStringLiteral("0")).rgb(), tokens::color(tokens::ink).rgb());
        TextRecorder changed;
        x->render(&changed);
        QCOMPARE(changed.colorOf(QStringLiteral("108")).rgb(), tokens::color(tokens::ink).rgb());
        QCOMPARE(changed.colorOf(QStringLiteral("px")).rgb(), tokens::color(tokens::ink3).rgb());
        QCOMPARE(changed.colorOf(QStringLiteral("X")).rgb(), tokens::color(tokens::ink3).rgb());
        QCOMPARE(QFontInfo(x->valueFont()).pixelSize(), 12);
    }

    // Double-clicking a row's label sends null for its value. A group's
    // reset (shown while the pointer is over its header, live once anything
    // in it moved) resets all its rows in one edit; its header's tooltip says
    // what changed in it. Folds are kept per pane and kind of clip; ⌥-click
    // opens or closes every group, ⌘-click opens only this one.
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
        auto *scaleRow = rowOf(inspector->findChild<ScrubField *>(QStringLiteral("prop_scale")));
        auto *opacityRow = rowOf(inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity")));
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
        // Double-clicking the label resets it.
        QMouseEvent doubleClick(QEvent::MouseButtonDblClick, QPointF(4, 4), scaleRow->label()->mapToGlobal(QPointF(4, 4)),
                                Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(scaleRow->label(), &doubleClick);
        QJsonObject op = takeOps(window).first().toObject();
        QVERIFY(op.value("props").toObject().contains("scale") && op.value("props").toObject().value("scale").isNull());
        // Crop says what changed in it, and resets in one edit.
        InspectorSection *crop = inspector->section(QStringLiteral("crop"));
        QVERIFY(crop && crop->isOpen());
        QCOMPARE(crop->summary(), QStringLiteral("Left 192 px · Bottom 54 px"));
        QVERIFY(crop->resetButton()->isHidden());
        hover(crop->header(), true);
        QVERIFY(crop->resetButton()->isVisibleTo(crop) && crop->resetButton()->isEnabled());
        crop->resetButton()->click();
        const QJsonArray ops = takeOps(window);
        QCOMPARE(ops.size(), 2);
        for (const QJsonValue &value : ops) {
            const QJsonObject props = value.toObject().value("props").toObject();
            QCOMPARE(props.size(), 1);
            QVERIFY(props.begin().value().isNull());
        }
        hover(crop->header(), false);
        QVERIFY(crop->resetButton()->isHidden());
        snapshot(2, QJsonObject{{"crop_b", 0.05}});
        QCOMPARE(crop->summary(), QStringLiteral("Bottom 54 px"));
        snapshot(3, {});
        QVERIFY(crop->summary().isEmpty());
        hover(crop->header(), true);
        QVERIFY(!crop->resetButton()->isEnabled());
        // A group folds and opens, and stays so for this pane and kind.
        crop->setOpen(false);
        emit crop->toggled(false, Qt::NoModifier);
        QVERIFY(!QSettings().value(QStringLiteral("editor/inspector/folds/video:video")).toMap().value(QStringLiteral("crop")).toBool());
        // ⌥-click opens or closes every group; ⌘-click opens only this one.
        InspectorSection *transform = inspector->section(QStringLiteral("transform"));
        InspectorSection *time = inspector->section(QStringLiteral("time"));
        QVERIFY(transform && time);
        emit crop->toggled(false, Qt::AltModifier);
        QVERIFY(!transform->isOpen() && !time->isOpen());
        transform->setOpen(true);
        emit transform->toggled(true, Qt::AltModifier);
        QVERIFY(crop->isOpen() && time->isOpen());
        emit transform->toggled(true, Qt::ControlModifier);
        QVERIFY(transform->isOpen() && !crop->isOpen() && !time->isOpen());
        // Folded, a group's rows hide and its chevron turns.
        QVERIFY(crop->bodyWidget()->isHidden());
        QVERIFY(!crop->chevron()->isOpen());
    }

    // Each group's fold is kept by id per pane and kind, so one never seen
    // opens or folds as the design has it: Effects, Compositing, Transform
    // and Crop open, the rest folded.
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
        QCOMPARE(opened(*build({QStringLiteral("v")})), (QStringList{"effects", "compositing", "transform", "crop"}));
        // The sequence: Format and Tracks open, Markers folded.
        {
            const auto inspector = build({});
            QVERIFY(inspector->section(QStringLiteral("tracks"))->isOpen());
            QVERIFY(!inspector->section(QStringLiteral("markers"))->isOpen());
        }
        // A fold is kept by id; a group never seen opens as it does by default.
        QSettings().setValue(QStringLiteral("editor/inspector/folds/video:video"),
                             QVariantMap{{QStringLiteral("crop"), true}, {QStringLiteral("compositing"), false}});
        {
            const auto inspector = build({QStringLiteral("v")});
            QCOMPARE(opened(*inspector), (QStringList{"effects", "transform", "crop"}));
            InspectorSection *transform = inspector->section(QStringLiteral("transform"));
            transform->setOpen(false);
            emit transform->toggled(false, Qt::NoModifier);
        }
        const QVariantMap folds = QSettings().value(QStringLiteral("editor/inspector/folds/video:video")).toMap();
        QCOMPARE(folds.value(QStringLiteral("transform")), QVariant(false));
        QCOMPARE(folds.value(QStringLiteral("crop")), QVariant(true));
        QCOMPARE(opened(*build({QStringLiteral("v")})), (QStringList{"effects", "crop"}));
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
        // Fit and Fill: a pop-up at the Transform group's header.
        auto *fit = inspector->findChild<QComboBox *>(QStringLiteral("fitFill"));
        QVERIFY(fit);
        QVERIFY(inspector->section(QStringLiteral("transform"))->header()->isAncestorOf(fit));
        QCOMPARE(fit->currentIndex(), 0);
        emit fit->activated(1);
        QJsonObject props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY2(std::abs(props.value("scale").toDouble() - 3.1605) < 0.001, qPrintable(QString::number(props.value("scale").toDouble())));
        QVERIFY(props.contains("x") && props.value("x").isNull() && props.contains("y") && props.value("y").isNull());
        snapshot(2, QJsonObject{{"scale", 3.1605}}, 1.0, false);
        QCOMPARE(fit->currentIndex(), 1);
        emit fit->activated(0);
        props = takeOps(window).first().toObject().value("props").toObject();
        QVERIFY(props.value("scale").isNull() && props.value("x").isNull());
        snapshot(3, QJsonObject{{"scale", 3.1605}, {"x", 0.1}}, 1.0, false);
        QCOMPARE(fit->currentIndex(), -1);
        QCOMPARE(fit->placeholderText(), QStringLiteral("Custom"));
        // Speed: 200% is a ratio of 2; Reverse its sign; presets keep the direction.
        ScrubField *speed = inspector->findChild<ScrubField *>(QStringLiteral("speed"));
        auto *reverse = inspector->findChild<GlyphButton *>(QStringLiteral("speedReverse"));
        QVERIFY(speed && reverse);
        QCOMPARE(speed->text(), QStringLiteral("100"));
        // Off, Reverse is an outline in the secondary ink, no fill.
        QVERIFY(!reverse->isChecked());
        QCOMPARE(reverse->inkColor(), tokens::color(tokens::ink2));
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
        // On: ink on the `on` fill (.ib.on), never the accent.
        QCOMPARE(reverse->inkColor(), tokens::color(tokens::ink));
        QCOMPARE(reverse->discColor(), tokens::color(tokens::on));
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

    // Selecting a clip shows what it is in the clip header: its name over
    // the mono line "track · in–out · duration"; its Start, End and Duration
    // in timecode in the Time group; its sound's volume in the Audio pane.
    void inspectorShowsWhatEachClipIs() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        InspectorPanel *inspector = window.m_inspector;
        const struct {
            const char *id;
            const char *meta;
            const char *start;
            const char *duration;
        } cases[] = {
            {"v", "V1 · 00:00:06:00\u201300:00:11:00 · 5s0f", "00:00:06:00", "00:00:05:00"},
            {"n", "V1 · 00:00:11:00\u201300:00:15:00 · 4s0f", "00:00:11:00", "00:00:04:00"},
            {"t", "V2 · 00:00:01:00\u201300:00:05:00 · 4s0f", "00:00:01:00", "00:00:04:00"},
            {"s", "V2 · 00:00:05:00\u201300:00:07:00 · 2s0f", "00:00:05:00", "00:00:02:00"},
            {"adj", "V2 · 00:00:08:00\u201300:00:11:00 · 3s0f", "00:00:08:00", "00:00:03:00"},
            // As the timeline's header names it: the subtitle track is the
            // third picture track, V3.
            {"cap", "V3 · 00:00:02:00\u201300:00:06:00 · 4s0f", "00:00:02:00", "00:00:04:00"},
            {"a", "A1 · 00:00:06:00\u201300:00:11:00 · 5s0f", "00:00:06:00", "00:00:05:00"},
            {"music", "A2 · 00:00:00:00\u201300:00:18:00 · 18s0f", "00:00:00:00", "00:00:18:00"},
        };
        for (const auto &expected : cases) {
            const QString id = QString::fromLatin1(expected.id);
            window.selectClip(id);
            QCOMPARE(inspector->metaText(), QString::fromUtf8(expected.meta));
            QCOMPARE(inspector->kindText(), inspector->metaText());
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_start"))->text(), QString::fromLatin1(expected.start));
            QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("timing_duration"))->text(), QString::fromLatin1(expected.duration));
            QVERIFY(!inspector->nameText().isEmpty());
        }
        // The adjustment says what it is for; its effects come first, open.
        window.selectClip(QStringLiteral("adj"));
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Applies its effects to every layer beneath it."));
        QVERIFY(!inspector->m_note->isHidden());
        QVERIFY(inspector->section(QStringLiteral("effects"))->isOpen());
        QCOMPARE(inspector->m_sections.first().section->id(), QStringLiteral("effects"));
        // The caption: its text, and how to change it.
        window.selectClip(QStringLiteral("cap"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("captionText"))->text(), QStringLiteral("We open on the harbor at dawn."));
        auto *captionAsk = inspector->findChild<QPushButton *>(QStringLiteral("captionHint"));
        QVERIFY(captionAsk);
        QCOMPARE(captionAsk->cursor().shape(), Qt::PointingHandCursor);
        QCOMPARE(inspector->m_sections.first().section->id(), QStringLiteral("caption"));
        // The video's sound (linked A1) is edited in the Audio pane, on the linked clip.
        window.selectClip(QStringLiteral("v"));
        inspector->setPane(InspectorPanel::Pane::Audio);
        QCOMPARE(inspector->metaText(), QStringLiteral("A1 linked · 48 kHz · stereo"));
        inspector->findChild<ScrubField *>(QStringLiteral("prop_gain"))->commitText(QStringLiteral("-8"));
        const QJsonObject op = takeOps(window).first().toObject();
        QCOMPARE(op.value("clip").toString(), QStringLiteral("a"));
        QCOMPARE(op.value("props").toObject().value("gain").toDouble(), -8.0);
        inspector->setPane(InspectorPanel::Pane::Video);
        // The music's volume reads in dB; the sound effects are offered.
        window.selectClip(QStringLiteral("music"));
        QCOMPARE(inspector->findChild<ScrubField *>(QStringLiteral("prop_gain"))->text(), QString(QChar(0x2212)) + QStringLiteral("8.0"));
        QVERIFY(inspector->section(QStringLiteral("volume")));
        QVERIFY(inspector->section(QStringLiteral("effects")));
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
        // ⋯ ▸ Ask Kerf about This Clip: the conversation, about it.
        QAction *ask = inspector->m_moreMenu->findChild<QAction *>(QStringLiteral("inspectorAsk"));
        QVERIFY(ask);
        ask->trigger();
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("@fisher-close.mov "));
        window.selectClip(QString());
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
        // Its frame, rate and length on the header's mono line.
        QCOMPARE(inspector->metaText(), QStringLiteral("1080×1920 · 24p · 00:00:18:00"));
        QVERIFY(inspector->findChildren<ScrubField *>().isEmpty());
        QCOMPARE(inspector->m_note->text(), QStringLiteral("Select a clip to adjust it."));
        QVERIFY(!inspector->m_note->isHidden());
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
        {
            // A sequence at another rate (as a project opens over an empty
            // state) shows its own rate, not the one the rows were built at.
            QJsonObject other = snapshot;
            QJsonObject timeline = other.value("timeline").toObject();
            timeline.insert(QStringLiteral("fps"), 30.0);
            other.insert(QStringLiteral("timeline"), timeline);
            const auto shownRate = [inspector] {
                for (QLabel *rate : inspector->findChildren<QLabel *>(QStringLiteral("sequenceRate")))
                    if (rate->parentWidget() && inspector->m_bodyWidget->isAncestorOf(rate)) return rate->text();
                return QString();
            };
            window.applySnapshot(other);
            QCOMPARE(shownRate(), QStringLiteral("30 fps"));
            window.applySnapshot(snapshot);
            QCOMPARE(shownRate(), QStringLiteral("24 fps"));
        }
        // Only the frame size can change; no clip fields.
        const auto popups = inspector->findChildren<QComboBox *>();
        QCOMPARE(popups.size(), 1);
        QComboBox *size = popups.first();
        QCOMPARE(size->objectName(), QStringLiteral("frameSize"));
        // Written as the header writes it (thin spaces round the ×).
        QCOMPARE(size->currentText(), QStringLiteral("9:16 · 1080\u2009×\u20091920"));
        QVERIFY(inspector->findChildren<QLineEdit *>().size() <= 1); // the name's editor, hidden
        QVERIFY(inspector->findChildren<QPlainTextEdit *>().isEmpty());
        for (InspectorSwitch *box : inspector->findChildren<InspectorSwitch *>()) QVERIFY(box->isHidden());
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
        QCOMPARE(inspector->kindText(), QStringLiteral("Video · V1"));
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
        // The timing values keep one alignment, timecode and "Mixed" alike.
        for (ScrubField *field : std::as_const(inspector->m_timingFields)) QCOMPARE(field->valueAlignment(), Qt::Alignment(Qt::AlignRight));
        // Different kinds: the timing only (the Time group), and why.
        inspector->setSelection({QStringLiteral("v1"), QStringLiteral("a1")});
        QCOMPARE(inspector->kindText(), QStringLiteral("1 Audio, 1 Video"));
        QCOMPARE(inspector->m_sections.size(), 1);
        QCOMPARE(inspector->m_sections.first().section->id(), QStringLiteral("time"));
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
        // Short enough for the open sections to scroll in the full-height sheet.
        window.resize(1440, 700);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.selectClip(QStringLiteral("c1"));
        window.m_shell->setSheet(StageShell::Sheet::Inspector);
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->isVisible());
        ScrubField *opacity = inspector->findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QCOMPARE(opacity->text(), QStringLiteral("50"));
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        inspector->section(QStringLiteral("crop"))->setOpen(true);
        QCoreApplication::processEvents();
        QScrollBar *scroll = inspector->scrollArea()->verticalScrollBar();
        // The body scrolls with its scroll bar hidden (mockup-spec 5).
        QCOMPARE(inspector->scrollArea()->verticalScrollBarPolicy(), Qt::ScrollBarAlwaysOff);
        QVERIFY(!scroll->isVisible());
        QVERIFY(scroll->maximum() > 60);
        scroll->setValue(60);
        // From the click to the painted values (the panel repainted at once,
        // not when the event loop gets to it).
        const auto timedSwitch = [&](const char *id) {
            QElapsedTimer timer;
            timer.start();
            window.selectClip(QString::fromLatin1(id));
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
        // The 12-point fade at the body's foot shows while there is more
        // below, over the viewport's last 12 points.
        QVERIFY(inspector->m_fade->isVisible());
        const QRect viewport = inspector->scrollArea()->viewport()->geometry();
        QCOMPARE(inspector->m_fade->geometry(), QRect(viewport.left(), viewport.bottom() + 1 - 12, viewport.width(), 12));
        scroll->setValue(scroll->maximum());
        QVERIFY(!inspector->m_fade->isVisible());
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
        window.selectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        const QStringList titleRows = visibleRows();
        const QString titleName = inspector->nameText();
        QVERIFY(!titleRows.isEmpty());
        window.selectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        const QStringList videoRows = visibleRows();
        QVERIFY(videoRows != titleRows);
        for (int round = 0; round < 2; ++round) {
            window.selectClip(QStringLiteral("t"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), titleRows);
            QCOMPARE(inspector->nameText(), titleName);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
            window.selectClip(QStringLiteral("v"));
            QCoreApplication::processEvents();
            QCOMPARE(visibleRows(), videoRows);
            QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
        }
        // A font or screen density change (EditorAppearance::changed)
        // builds every body again.
        emit EditorAppearance::instance()->changed();
        window.selectClip(QStringLiteral("t"));
        QCoreApplication::processEvents();
        QCOMPARE(visibleRows(), titleRows);
        QCOMPARE(inspector->findChildren<QWidget *>(QStringLiteral("inspectorBody")).size(), 1);
    }

    // Rotation is a bipolar slider now (the knob is retired), with its edit
    // semantics kept: a stray click turns nothing, a drag is one edit, and a
    // double-click on the knob puts every selected clip back at 0°, not the
    // second shifted by the first's turn.
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
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        inspector.setState(timeline, {QStringLiteral("v1")});
        inspector.section(QStringLiteral("transform"))->setOpen(true);
        inspector.resize(304, 900);
        inspector.show();
        QVERIFY(QTest::qWaitForWindowExposed(&inspector));
        auto *slider = inspector.findChild<InspectorSlider *>(QStringLiteral("slider_rotation"));
        QVERIFY(slider && slider->isVisible());
        // Bipolar about 0°: the tick at its middle, filled from there.
        QVERIFY(slider->isBipolar());
        QVERIFY(!slider->detentTick().isEmpty());
        QCOMPARE(slider->value(), 30.0);
        const double knob = slider->knobCenter().x();
        // A click that does not move turns nothing.
        mouse(slider, QEvent::MouseButtonPress, QPointF(knob, 12));
        mouse(slider, QEvent::MouseButtonRelease, QPointF(knob, 12));
        QVERIFY(sent.isEmpty());
        // A drag: one edit, to the value the knob reached.
        mouse(slider, QEvent::MouseButtonPress, QPointF(knob, 12));
        mouse(slider, QEvent::MouseMove, QPointF(knob + 10, 12));
        mouse(slider, QEvent::MouseButtonRelease, QPointF(knob + 10, 12));
        QCOMPARE(sent.size(), 1);
        const double turned = sent.last().first().toObject().value("props").toObject().value("rotation").toDouble();
        QVERIFY2(turned > 30.0 && turned < 180.0, qPrintable(QString::number(turned)));
        // Two clips at 30° and 10: a double-click on the knob resets both.
        sent.clear();
        inspector.setState(timeline, {QStringLiteral("v1"), QStringLiteral("v2")});
        slider = inspector.findChild<InspectorSlider *>(QStringLiteral("slider_rotation"));
        QVERIFY(slider);
        // Mixed, the knob hides; the row's label double-click resets them.
        RowLabel *label = rowOf(slider)->label();
        QTest::mouseDClick(label, Qt::LeftButton);
        QCOMPARE(sent.size(), 1);
        QStringList reset;
        for (const QJsonValue &op : sent.last()) {
            QVERIFY(op.toObject().value("props").toObject().value("rotation").isNull());
            reset << op.toObject().value("clip").toString();
        }
        QCOMPARE(reset, (QStringList{"v1", "v2"}));
    }

    // Keyframes are real: a number row's diamond writes {keys:[{at: frame,
    // value, interp}]} at the playhead (clip-relative frames, as kerfd keys
    // them); a key under the playhead is the filled diamond, "Keyframe at
    // playhead", and a click takes it out (the last one leaves the value it
    // had, still); editing an animated value sets the key at the playhead.
    // Sound keys only on gain; outside the clip there is no key to set.
    void inspectorKeyframesToggleAtThePlayhead() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        const auto withProps = [&timeline](const char *clipId, const QJsonObject &props) {
            QJsonArray tracks = timeline.value("tracks").toArray();
            for (int t = 0; t < tracks.size(); ++t) {
                QJsonObject track = tracks.at(t).toObject();
                QJsonArray clips = track.value("clips").toArray();
                for (int c = 0; c < clips.size(); ++c) {
                    QJsonObject clip = clips.at(c).toObject();
                    if (clip.value("id").toString() != QLatin1String(clipId)) continue;
                    clip.insert("props", props);
                    clips.replace(c, clip);
                }
                track.insert("clips", clips);
                tracks.replace(t, track);
            }
            QJsonObject out = timeline;
            out.insert("tracks", tracks);
            return out;
        };
        InspectorPanel inspector;
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        // The close-up "v" runs 6 s to 11 s (frames 144-264); the playhead at 6.5 s is its frame 12.
        inspector.setPlayhead(6.5);
        inspector.setState(withProps("v", QJsonObject{{"opacity", 0.5}}), {QStringLiteral("v")});
        const auto rowOf = [&inspector](const char *field) -> InspectorRow * {
            for (QWidget *at = inspector.findChild<QWidget *>(QString::fromLatin1(field)); at; at = at->parentWidget())
                if (auto *row = qobject_cast<InspectorRow *>(at)) return row;
            return nullptr;
        };
        InspectorRow *opacity = rowOf("prop_opacity");
        QVERIFY(opacity);
        KeyframeButton *diamond = opacity->keyframeButton();
        QVERIFY(diamond->isEnabled() && !diamond->has());
        QCOMPARE(diamond->iconName(), QStringLiteral("g-kf"));
        QCOMPARE(ui::tipName(diamond), QStringLiteral("Add keyframe"));
        diamond->click();
        QCOMPARE(sent.size(), 1);
        QJsonObject op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("v"));
        const QJsonArray keys = op.value("props").toObject().value("opacity").toObject().value("keys").toArray();
        QCOMPARE(keys.size(), 1);
        QCOMPARE(keys.first().toObject().value("at").toInt(), 12);
        QCOMPARE(keys.first().toObject().value("value").toDouble(), 0.5);
        QCOMPARE(keys.first().toObject().value("interp").toString(), QStringLiteral("linear"));
        // The snapshot with the key: the filled diamond, and its tooltip.
        const QJsonArray keyed{QJsonObject{{"at", 0.5}, {"frame", 12}, {"value", 0.5}, {"interp", "linear"}}};
        inspector.setState(withProps("v", QJsonObject{{"opacity", keyed}}), {QStringLiteral("v")});
        diamond = rowOf("prop_opacity")->keyframeButton();
        QVERIFY(diamond->has());
        QCOMPARE(diamond->iconName(), QStringLiteral("g-kff"));
        QCOMPARE(ui::tipName(diamond), QStringLiteral("Keyframe at playhead"));
        // Moved a second on, the value edits the key there: two keys.
        sent.clear();
        inspector.setPlayhead(7.5);
        QVERIFY(!diamond->has());
        auto *field = inspector.findChild<ScrubField *>(QStringLiteral("prop_opacity"));
        QVERIFY(field && !field->isReadOnly());
        field->commitText(QStringLiteral("80"));
        QCOMPARE(sent.size(), 1);
        const QJsonArray two = sent.last().first().toObject().value("props").toObject().value("opacity").toObject().value("keys").toArray();
        QCOMPARE(two.size(), 2);
        QCOMPARE(two.at(0).toObject().value("at").toInt(), 12);
        QCOMPARE(two.at(1).toObject().value("at").toInt(), 36);
        QVERIFY(std::abs(two.at(1).toObject().value("value").toDouble() - 0.8) < 1e-9);
        // Back on the key, a click takes it out: the last one leaves 0.5, still.
        sent.clear();
        inspector.setPlayhead(6.5);
        QVERIFY(diamond->has());
        diamond->click();
        QCOMPARE(sent.size(), 1);
        QCOMPARE(sent.last().first().toObject().value("props").toObject().value("opacity").toDouble(), 0.5);
        // Outside the clip: nothing to key, and why.
        inspector.setPlayhead(2.0);
        QVERIFY(!diamond->isEnabled());
        QVERIFY(ui::tipName(diamond).contains(QStringLiteral("playhead over the clip")));
        // Sound: gain keys, pan does not ("Playback and renders follow
        // keyframes on gain only").
        inspector.setPlayhead(1.0);
        inspector.setState(timeline, {QStringLiteral("music")});
        InspectorRow *gain = rowOf("prop_gain");
        InspectorRow *pan = rowOf("prop_pan");
        QVERIFY(gain && pan);
        QVERIFY(gain->keyframeButton()->isEnabled());
        QVERIFY(!pan->keyframeButton()->isEnabled());
        QCOMPARE(ui::tipName(pan->keyframeButton()), QStringLiteral("Playback and renders follow keyframes on gain only"));
        sent.clear();
        gain->keyframeButton()->click();
        QCOMPARE(sent.size(), 1);
        const QJsonArray gainKeys = sent.last().first().toObject().value("props").toObject().value("gain").toObject().value("keys").toArray();
        QCOMPARE(gainKeys.size(), 1);
        QCOMPARE(gainKeys.first().toObject().value("at").toInt(), 24);
        QCOMPARE(gainKeys.first().toObject().value("value").toDouble(), -8.0);
        // A value that cannot be animated (blend) has no diamond.
        inspector.setPlayhead(6.5);
        inspector.setState(timeline, {QStringLiteral("v")});
        QVERIFY(rowOf("prop_blend")->keyframeButton()->isHidden());
    }

    // The Finish workspace's Lift, Gamma and Gain wheels (72 points, a 2pt
    // ring, a 48pt crosshair, an 8pt puck) write the clip's ASC CDL: offset,
    // power and slope per channel, added on the first edit; the master value
    // under each is the luma (Lift −0.02, Gamma +0.01, Gain 1.04); with only
    // Lumetri on the clip they write its wheels instead.
    void inspectorWheelsWriteTheAscCdl() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        const auto withEffects = [&timeline](const QJsonArray &effects) {
            QJsonArray tracks = timeline.value("tracks").toArray();
            QJsonObject track = tracks.at(0).toObject();
            QJsonArray clips = track.value("clips").toArray();
            QJsonObject clip = clips.at(0).toObject();
            clip.insert("effects", effects);
            clips.replace(0, clip);
            track.insert("clips", clips);
            tracks.replace(0, track);
            QJsonObject out = timeline;
            out.insert("tracks", tracks);
            return out;
        };
        InspectorPanel inspector;
        char *catalog = kerf_fx_catalog_all();
        QVERIFY(catalog);
        inspector.setEffectCatalog(QJsonDocument::fromJson(QByteArray(catalog)).array());
        kerf_bridge_string_free(catalog);
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        inspector.setState(withEffects({}), {QStringLiteral("v")});
        // Not in Finish: no wheels.
        QVERIFY(!inspector.wheels());
        inspector.setFinish(true);
        QVERIFY(inspector.wheels());
        inspector.resize(304, 900);
        inspector.show();
        QVERIFY(QTest::qWaitForWindowExposed(&inspector));
        const auto wheels = inspector.wheels()->findChildren<ColorWheel *>();
        QCOMPARE(wheels.size(), 3);
        for (ColorWheel *wheel : wheels) {
            QCOMPARE(wheel->size(), QSize(72, 72));
            QCOMPARE(wheel->puckCenter(), QPointF(36, 36));
        }
        // Spread across the column, 16 in from its sides, under the header.
        QCOMPARE(wheels.first()->mapTo(&inspector, QPoint()).x(), 16);
        QCOMPARE(wheels.last()->mapTo(&inspector, QPoint(72, 0)).x(), inspector.width() - 16);
        QCOMPARE(inspector.wheels()->mapTo(&inspector, QPoint()).y(), 44);
        auto *gain = inspector.findChild<ColorWheel *>(QStringLiteral("colorWheel_gain"));
        auto *gainValue = inspector.findChild<ScrubField *>(QStringLiteral("wheelValue_gain"));
        auto *liftValue = inspector.findChild<ScrubField *>(QStringLiteral("wheelValue_lift"));
        QVERIFY(gain && gainValue && liftValue);
        QCOMPARE(gainValue->text(), QStringLiteral("1.00"));
        QCOMPARE(liftValue->text(), QStringLiteral("0.00"));
        // A drag right pushes blue (Cb): the CDL is added with the slopes.
        mouse(gain, QEvent::MouseButtonPress, QPointF(36, 36));
        mouse(gain, QEvent::MouseMove, QPointF(46, 36));
        mouse(gain, QEvent::MouseButtonRelease, QPointF(46, 36));
        QCOMPARE(sent.size(), 1);
        QJsonObject op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(op.value("fx").toString(), QStringLiteral("color.asccdl"));
        const QJsonObject added = op.value("params").toObject();
        QVERIFY2(added.value("b_slope").toDouble() > 1.0, qPrintable(QJsonDocument(added).toJson(QJsonDocument::Compact)));
        QVERIFY(added.value("g_slope").toDouble() < 1.0);
        // On the clip: its values show, and a typed Gain sets every slope.
        sent.clear();
        inspector.setState(withEffects({QJsonObject{{"fx", "color.asccdl"}, {"enabled", true},
                                                    {"params", QJsonObject{{"r_offset", -0.02}, {"g_offset", -0.02}, {"b_offset", -0.02},
                                                                           {"r_power", 1.01}, {"g_power", 1.01}, {"b_power", 1.01}}}}}),
                           {QStringLiteral("v")});
        // A new stack is a new body: its wheels found again.
        gainValue = inspector.findChild<ScrubField *>(QStringLiteral("wheelValue_gain"));
        liftValue = inspector.findChild<ScrubField *>(QStringLiteral("wheelValue_lift"));
        QVERIFY(gainValue && liftValue);
        QCOMPARE(liftValue->text(), QStringLiteral("−0.02"));
        QCOMPARE(inspector.findChild<ScrubField *>(QStringLiteral("wheelValue_gamma"))->text(), QStringLiteral("+0.01"));
        gainValue->commitText(QStringLiteral("1.04"));
        QCOMPARE(sent.size(), 1);
        op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(op.value("index").toInt(), 0);
        for (const char *slope : {"r_slope", "g_slope", "b_slope"})
            QVERIFY2(std::abs(op.value("params").toObject().value(QLatin1String(slope)).toDouble() - 1.04) < 1e-4, slope);
        // A double-click puts the puck back: the chroma goes, the level stays.
        sent.clear();
        auto *lift = inspector.findChild<ColorWheel *>(QStringLiteral("colorWheel_lift"));
        QTest::mouseDClick(lift, Qt::LeftButton, Qt::NoModifier, QPoint(36, 36));
        // Neutral already: nothing to send.
        QVERIFY(sent.isEmpty());
        // Lumetri alone grades: the wheels are its own.
        inspector.setState(withEffects({QJsonObject{{"fx", "color.lumetri"}, {"enabled", true}, {"params", QJsonObject{}}}}),
                           {QStringLiteral("v")});
        lift = inspector.findChild<ColorWheel *>(QStringLiteral("colorWheel_lift"));
        mouse(lift, QEvent::MouseButtonPress, QPointF(36, 36));
        mouse(lift, QEvent::MouseMove, QPointF(36, 50));
        mouse(lift, QEvent::MouseButtonRelease, QPointF(36, 50));
        QCOMPARE(sent.size(), 1);
        op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-set"));
        QVERIFY(op.value("params").toObject().value("wheel_shadows_y").toDouble() > 0.0);
        // Out of Finish they go.
        inspector.setFinish(false);
        QVERIFY(!inspector.wheels());
    }

    // The Audio pane: Volume (Level in dB, Pan bipolar) and Channel EQ,
    // audio.eq's low cut and presence, added on the first edit (its box off
    // until it is on the clip). A picture clip's pane edits its linked sound;
    // one with none says so.
    void inspectorAudioPaneAddsTheChannelEq() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        InspectorPanel inspector;
        char *catalog = kerf_fx_catalog_all();
        QVERIFY(catalog);
        inspector.setEffectCatalog(QJsonDocument::fromJson(QByteArray(catalog)).array());
        kerf_bridge_string_free(catalog);
        QList<QJsonArray> sent;
        connect(&inspector, &InspectorPanel::editRequested, &inspector, [&sent](const QJsonArray &ops) { sent << ops; });
        inspector.setPane(InspectorPanel::Pane::Audio);
        inspector.setState(timeline, {QStringLiteral("music")});
        InspectorSection *volume = inspector.section(QStringLiteral("volume"));
        InspectorSection *eq = inspector.section(QStringLiteral("channelEq"));
        QVERIFY(volume && eq);
        QVERIFY(volume->isOpen() && eq->isOpen());
        QCOMPARE(eq->title(), QStringLiteral("Channel EQ"));
        QCOMPARE(eq->enable(), InspectorSection::Enable::Off);
        QVERIFY(inspector.metaText().startsWith(QStringLiteral("A2 · 48 kHz")));
        QVERIFY(inspector.metaText().endsWith(QStringLiteral("stereo")));
        auto *level = inspector.findChild<ScrubField *>(QStringLiteral("prop_gain"));
        auto *pan = inspector.findChild<InspectorSlider *>(QStringLiteral("slider_pan"));
        auto *lowCut = inspector.findChild<ScrubField *>(QStringLiteral("fxParam_lowcut_hz"));
        auto *presence = inspector.findChild<ScrubField *>(QStringLiteral("fxParam_presence_db"));
        QVERIFY(level && pan && lowCut && presence);
        QVERIFY(pan->isBipolar());
        QCOMPARE(level->text(), QStringLiteral("−8.0"));
        QCOMPARE(lowCut->unitText(), QStringLiteral("Hz"));
        // The labels read as the design's.
        QCOMPARE(rowOf(lowCut)->label()->fullText(), QStringLiteral("Low cut"));
        QCOMPARE(rowOf(presence)->label()->fullText(), QStringLiteral("Presence"));
        // The first edit adds audio.eq with its value.
        lowCut->commitText(QStringLiteral("80"));
        QCOMPARE(sent.size(), 1);
        QJsonObject op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("music"));
        QCOMPARE(op.value("fx").toString(), QStringLiteral("audio.eq"));
        QCOMPARE(op.value("params").toObject().value("lowcut_hz").toDouble(), 80.0);
        // Checking the box adds it plain.
        sent.clear();
        eq->enableBox()->click();
        QCOMPARE(sent.size(), 1);
        QCOMPARE(sent.last().first().toObject().value("fx").toString(), QStringLiteral("audio.eq"));
        // On the clip, edits set it, and its box switches it.
        QJsonArray tracks = timeline.value("tracks").toArray();
        QJsonObject music = tracks.at(4).toObject();
        QJsonArray clips = music.value("clips").toArray();
        QJsonObject clip = clips.at(0).toObject();
        clip.insert("effects", QJsonArray{QJsonObject{{"fx", "audio.eq"}, {"enabled", true}, {"params", QJsonObject{{"lowcut_hz", 80}}}}});
        clips.replace(0, clip);
        music.insert("clips", clips);
        tracks.replace(4, music);
        timeline.insert("tracks", tracks);
        sent.clear();
        inspector.setState(timeline, {QStringLiteral("music")});
        eq = inspector.section(QStringLiteral("channelEq"));
        QVERIFY(eq);
        QCOMPARE(eq->enable(), InspectorSection::Enable::On);
        presence = inspector.findChild<ScrubField *>(QStringLiteral("fxParam_presence_db"));
        presence->commitText(QStringLiteral("1.5"));
        op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(op.value("index").toInt(), 0);
        QCOMPARE(op.value("params").toObject().value("presence_db").toDouble(), 1.5);
        inspector.section(QStringLiteral("channelEq"))->enableBox()->click();
        op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(op.value("enabled").toBool(), false);
        // Sound effect parameters take no keyframes.
        QVERIFY(!rowOf(presence)->keyframeButton()->isEnabled());
        // A level typed: the clip's gain.
        level = inspector.findChild<ScrubField *>(QStringLiteral("prop_gain"));
        level->commitText(QStringLiteral("-6"));
        op = sent.last().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("props").toObject().value("gain").toDouble(), -6.0);
        // The picture clip "v": its linked sound "a" on A1.
        sent.clear();
        inspector.setState(timeline, {QStringLiteral("v")});
        QVERIFY(inspector.metaText().startsWith(QStringLiteral("A1 linked")));
        inspector.findChild<ScrubField *>(QStringLiteral("prop_gain"))->commitText(QStringLiteral("-3"));
        QCOMPARE(sent.last().first().toObject().value("clip").toString(), QStringLiteral("a"));
        // A title carries no sound.
        inspector.setState(timeline, {QStringLiteral("t")});
        QVERIFY(!inspector.section(QStringLiteral("volume")));
        QVERIFY(inspector.findChild<QLabel *>(QStringLiteral("inspectorNote"))->text().contains(QStringLiteral("no sound")));
    }

    // The Info pane: key-value rows from the snapshot (Start, End, Duration,
    // Frame size, Frame rate, Roles from the track) and from media_info
    // (Codec, Colour, Camera, Lens, Reel); a field the engine does not give is
    // left out, never invented.
    void inspectorInfoShowsMediaInfoOnlyWhenPresent() {
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        QJsonArray tracks = timeline.value("tracks").toArray();
        QJsonObject v1 = tracks.at(0).toObject();
        v1.insert("role", "video");
        tracks.replace(0, v1);
        timeline.insert("tracks", tracks);
        InspectorPanel inspector;
        inspector.setPane(InspectorPanel::Pane::Info);
        inspector.setState(timeline, {QStringLiteral("v")});
        QCOMPARE(inspector.infoKeys(), (QStringList{"Start", "End", "Duration", "Frame size", "Frame rate", "Audio", "Roles", "File"}));
        QCOMPARE(inspector.infoValue(QStringLiteral("Start")), QStringLiteral("00:00:06:00"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Duration")), QStringLiteral("00:00:05:00"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Frame size")), QStringLiteral("1920×1080"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Frame rate")), QStringLiteral("24p"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Roles")), QStringLiteral("Video"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Codec")), QString());
        // Info on the file: only the fields it gives.
        inspector.setMediaInfo(QJsonObject{{"close", QJsonObject{{"codec", "prores"}, {"profile", "4444"}, {"camera", "ARRI ALEXA Mini LF"},
                                                                 {"lens", "Signature Prime 47 mm"}, {"reel", "B003"}, {"primaries", "bt709"},
                                                                 {"transfer", "bt709"}, {"audioBitDepth", 24}}}});
        QCOMPARE(inspector.infoValue(QStringLiteral("Codec")), QStringLiteral("ProRes 4444"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Lens")), QStringLiteral("Signature Prime 47 mm"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Reel")), QStringLiteral("B003"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Colour")), QStringLiteral("Rec.709"));
        QCOMPARE(inspector.infoValue(QStringLiteral("Audio")), QStringLiteral("48 kHz · 24-bit · stereo"));
        QCOMPARE(inspector.metaText(), QStringLiteral("ARRI ALEXA Mini LF · ProRes 4444"));
        QVERIFY(!inspector.infoKeys().contains(QStringLiteral("Timecode")));
        QVERIFY(!inspector.infoKeys().contains(QStringLiteral("Pixel format")));
        // Each row: 24 tall, its value right-aligned in mono.
        for (QWidget *row : inspector.findChildren<QWidget *>(QStringLiteral("infoRow"))) QCOMPARE(row->height(), 24);
        auto *codec = inspector.findChildren<QLabel *>(QStringLiteral("infoValue")).value(0);
        QVERIFY(codec);
        QVERIFY(codec->alignment() & Qt::AlignRight);
        QVERIFY(QFontInfo(codec->font()).fixedPitch() || codec->font().family().contains(QStringLiteral("Mono")) ||
                codec->font().family() == QLatin1String(tokens::monoFallback));
        // The window takes media_info from the bridge's event.
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(2));
        window.handleEvent(QJsonObject{{"type", "media_info"}, {"media", QJsonObject{{"close", QJsonObject{{"lens", "Signature Prime 47 mm"}}}}}});
        QCOMPARE(window.m_inspector->mediaInfo().value("close").toObject().value("lens").toString(), QStringLiteral("Signature Prime 47 mm"));
    }

    // An open proposal that changes the clip shows as a hint row in the
    // group it changes (Effects for an effect): "<author> proposes <change>"
    // with Show, which opens the comments panel on the proposal's thread.
    void inspectorProposalHintOpensItsThread() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = everyKindSnapshot(3);
        QJsonObject proposed = snapshot.value("timeline").toObject();
        QJsonArray tracks = proposed.value("tracks").toArray();
        QJsonObject v1 = tracks.at(0).toObject();
        QJsonArray clips = v1.value("clips").toArray();
        QJsonObject clip = clips.at(0).toObject();
        clip.insert("effects", QJsonArray{QJsonObject{{"fx", "color.basic"}, {"enabled", true}, {"params", QJsonObject{}}}});
        clip.insert("props", QJsonObject{{"opacity", 0.5}});
        clips.replace(0, clip);
        v1.insert("clips", clips);
        tracks.replace(0, v1);
        proposed.insert("tracks", tracks);
        snapshot.insert("proposals", QJsonArray{QJsonObject{{"proposalId", "p1"}, {"status", "pending"},
                                                            {"author", QJsonObject{{"kind", "mcp"}, {"name", "Codex"}}}}});
        snapshot.insert("proposalTimelines", QJsonObject{{"p1", proposed}});
        window.applySnapshot(snapshot);
        window.handleEvent(QJsonObject{{"type", "comments"}, {"reviewRevision", 1}, {"items", QJsonArray{
            QJsonObject{{"id", "c-root"}, {"parent", QJsonValue()}, {"body", "Fade the close-up"}},
            QJsonObject{{"id", "c-reply"}, {"parent", "c-root"}, {"body", "Here it is"}, {"proposal", "p1"}}}}});
        window.selectClip(QStringLiteral("v"));
        InspectorPanel *inspector = window.m_inspector;
        QWidget *hint = inspector->proposalHint();
        QVERIFY(hint && !hint->isHidden());
        QVERIFY2(inspector->proposalHintText().startsWith(QStringLiteral("Codex proposes Opacity 50")), qPrintable(inspector->proposalHintText()));
        QVERIFY(inspector->proposalHintText().endsWith(QStringLiteral("+1")));
        QCOMPARE(hint->height(), 20);
        // In the group of what it changes: Compositing holds Opacity.
        QVERIFY(inspector->section(QStringLiteral("compositing"))->bodyWidget()->isAncestorOf(hint));
        QSignalSpy opened(window.m_bus, &EditorBus::openThread);
        hint->findChild<QPushButton *>(QStringLiteral("proposalHintShow"))->click();
        QCOMPARE(opened.size(), 1);
        QCOMPARE(opened.first().first().toString(), QStringLiteral("c-root"));
        // An effect change only: in Effects.
        clip.insert("props", QJsonObject{});
        clips.replace(0, clip);
        v1.insert("clips", clips);
        tracks.replace(0, v1);
        proposed.insert("tracks", tracks);
        snapshot.insert("revision", 4);
        snapshot.insert("proposalTimelines", QJsonObject{{"p1", proposed}});
        window.applySnapshot(snapshot);
        QVERIFY(!hint->isHidden());
        QVERIFY(inspector->section(QStringLiteral("effects"))->bodyWidget()->isAncestorOf(hint));
        QVERIFY(inspector->proposalHintText().startsWith(QStringLiteral("Codex proposes")));
        // Settled: no hint.
        snapshot.insert("revision", 5);
        snapshot.insert("proposals", QJsonArray{QJsonObject{{"proposalId", "p1"}, {"status", "accepted"}}});
        window.applySnapshot(snapshot);
        QVERIFY(hint->isHidden());
    }

    // The column's kinds are its panes, Audio and Info included; in the
    // Deliver workspace the Video kind shows the Deliver pane; a workspace
    // switch goes back to the Inspector page; Finish shows the wheels.
    void inspectorColumnShowsEachKindAndTheDeliverPane() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.selectClip(QStringLiteral("v"));
        InspectorColumn *column = window.m_inspectorColumn;
        InspectorPanel *inspector = window.m_inspector;
        for (const auto kind : {InspectorColumn::Kind::Video, InspectorColumn::Kind::Audio, InspectorColumn::Kind::Info})
            QVERIFY(InspectorColumn::kindAvailable(kind) && column->kindButton(kind)->isEnabled());
        column->kindButton(InspectorColumn::Kind::Audio)->click();
        QCOMPARE(column->kind(), InspectorColumn::Kind::Audio);
        QCOMPARE(inspector->pane(), InspectorPanel::Pane::Audio);
        QCOMPARE(column->currentPage(), inspector);
        QVERIFY(inspector->section(QStringLiteral("volume")));
        QVERIFY(inspector->footer()->isHidden());
        column->kindButton(InspectorColumn::Kind::Info)->click();
        QCOMPARE(inspector->pane(), InspectorPanel::Pane::Info);
        QVERIFY(inspector->infoKeys().contains(QStringLiteral("Frame size")));
        // Choosing a kind from the Source page turns the Inspector page back on.
        column->setPage(InspectorColumn::Page::Source);
        column->kindButton(InspectorColumn::Kind::Video)->click();
        QCOMPARE(column->page(), InspectorColumn::Page::Inspector);
        QCOMPARE(inspector->pane(), InspectorPanel::Pane::Video);
        QVERIFY(!inspector->footer()->isHidden());
        // Deliver: the Deliver pane stands in for the Video kind.
        window.m_shell->setTray(StageShell::Tray::Deliver);
        QCoreApplication::processEvents();
        QVERIFY(column->deliverShown());
        QCOMPARE(column->currentPage(), column->deliverPane());
        column->setKind(InspectorColumn::Kind::Info);
        QCOMPARE(column->currentPage(), inspector);
        column->setKind(InspectorColumn::Kind::Video);
        QCOMPARE(column->currentPage(), column->deliverPane());
        // Finish: the wheels over the Video pane.
        column->setPage(InspectorColumn::Page::Source);
        window.m_shell->setTray(StageShell::Tray::Finish);
        QCoreApplication::processEvents();
        QCOMPARE(column->page(), InspectorColumn::Page::Inspector);
        QVERIFY(inspector->finish());
        QVERIFY(inspector->wheels());
        window.m_shell->setTray(StageShell::Tray::None);
        QCoreApplication::processEvents();
        QVERIFY(!inspector->finish());
    }

    // Save effects preset (the footer, 32 tall with its line on top): a name,
    // then look_save {clip, name}; a bridge that answers look_save as an
    // unknown command turns the button off, saying why.
    void inspectorSavesAnEffectsPreset() {
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("editor/inspector")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = everyKindSnapshot(1);
        QJsonObject timeline = snapshot.value("timeline").toObject();
        QJsonArray tracks = timeline.value("tracks").toArray();
        QJsonObject v1 = tracks.at(0).toObject();
        QJsonArray clips = v1.value("clips").toArray();
        QJsonObject clip = clips.at(0).toObject();
        clip.insert("effects", QJsonArray{QJsonObject{{"fx", "color.basic"}, {"enabled", true}, {"params", QJsonObject{}}}});
        clips.replace(0, clip);
        v1.insert("clips", clips);
        tracks.replace(0, v1);
        timeline.insert("tracks", tracks);
        snapshot.insert("timeline", timeline);
        window.applySnapshot(snapshot);
        InspectorPanel *inspector = window.m_inspector;
        // Nothing selected: nothing to save.
        QVERIFY(!inspector->savePresetButton()->isEnabled());
        window.selectClip(QStringLiteral("v"));
        ui::Button *save = inspector->savePresetButton();
        QVERIFY(save->isEnabled());
        QCOMPARE(save->text(), QStringLiteral("Save effects preset"));
        QCOMPARE(save->kind(), ui::Button::Kind::Secondary);
        QCOMPARE(inspector->footer()->height(), 32);
        window.m_inspectorWiring->setNameAsker([](QWidget *, const QString &) { return QStringLiteral("Harbor warm"); });
        save->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("look_save"));
        QCOMPARE(window.m_lastCommand.value("clip").toString(), QStringLiteral("v"));
        QCOMPARE(window.m_lastCommand.value("name").toString(), QStringLiteral("Harbor warm"));
        // A cancelled name sends nothing.
        window.m_lastCommand = {};
        window.m_inspectorWiring->setNameAsker([](QWidget *, const QString &) { return QString(); });
        save->click();
        QVERIFY(window.m_lastCommand.isEmpty());
        // Refused as unknown: off, with the reason.
        window.handleEvent(QJsonObject{{"type", "error"}, {"command", "look_save"}, {"message", "unknown command: look_save"}});
        QVERIFY(!save->isEnabled());
        QCOMPARE(ui::tipName(save), InspectorWiring::unsupportedReason());
        window.handleEvent(QJsonObject{{"type", "look_saved"}, {"path", "looks/Harbor warm.kerflook"}});
        QVERIFY(save->isEnabled());
    }

    // The 304-point column at the design's 1600 × 1000 window, as spec 5
    // draws it: the 28 strip, the 44 clip header (name UI 500 14/20 over the
    // mono meta line), 28-point group headers, and 24-point rows on the
    // 84 | 1fr | 56 | 20 grid with 8 between, pad 0 8 0 12. With
    // KERF_SCREENSHOT_DIR set, each pane is saved as inspector-<pane>.png.
    void inspectorColumnFollowsTheSpecGrid() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.selectClip(QStringLiteral("v"));
        window.m_shell->setSheet(StageShell::Sheet::Inspector);
        QCoreApplication::processEvents();
        InspectorColumn *column = window.m_inspectorColumn;
        InspectorPanel *inspector = window.m_inspector;
        // 304 wide; its hairline stands outside it.
        QCOMPARE(column->width(), tokens::layout::inspector);
        QCOMPARE(column->findChild<QWidget *>(QStringLiteral("inspectorStrip"))->height(), 28);
        QCOMPARE(inspector->mapTo(column, QPoint()).y(), 28);
        QCOMPARE(inspector->header()->height(), 44);
        QCOMPARE(inspector->m_name->font().pixelSize(), 14);
        QCOMPARE(inspector->m_name->font().weight(), QFont::Medium);
        QCOMPARE(inspector->m_name->mapTo(inspector->header(), QPoint()), QPoint(12, 4));
        QCOMPARE(inspector->m_kind->height(), 16);
        QCOMPARE(inspector->m_kind->font().pixelSize(), 11);
        // "V1 · 00:00:06:00–00:00:11:00 · 5s0f"
        QCOMPARE(inspector->metaText(), QStringLiteral("V1 · 00:00:06:00–00:00:11:00 · 5s0f"));
        QCOMPARE(inspector->m_more->size(), QSize(24, 24));
        QCOMPARE(inspector->m_more->mapTo(inspector->header(), QPoint(24, 0)), QPoint(inspector->width() - 8, 4));
        InspectorSection *transform = inspector->section(QStringLiteral("transform"));
        QVERIFY(transform && transform->isOpen());
        QCOMPARE(transform->header()->height(), 28);
        QCOMPARE(transform->width(), inspector->width());
        // Every visible row on the one grid.
        int checked = 0;
        for (InspectorRow *row : inspector->findChildren<InspectorRow *>()) {
            if (!row->isVisibleTo(inspector) || row->isWide()) continue;
            ++checked;
            QCOMPARE(row->height(), 24);
            QCOMPARE(row->label()->geometry(), QRect(12, 0, 84, 24));
            QWidget *value = row->valueSlot();
            KeyframeButton *keyframe = row->keyframeButton();
            QCOMPARE(keyframe->geometry().right() + 1, row->width() - 8);
            QCOMPARE(keyframe->size(), QSize(20, 20));
            QCOMPARE(value->width(), 56);
            QCOMPARE(value->geometry().right() + 1 + 8, keyframe->x());
            QWidget *control = row->findChild<QWidget *>(QStringLiteral("rowControls"));
            QCOMPARE(control->x(), 12 + 84 + 8);
            QCOMPARE(control->geometry().right() + 1 + 8, value->x());
            QCOMPARE(control->width(), row->width() - 12 - 8 - 84 - 56 - 20 - 3 * 8);
        }
        QVERIFY2(checked >= 4, qPrintable(QString::number(checked)));
        // Wide rows (pairs, pop-ups) end where the value column does.
        for (const char *pair : {"prop_y", "prop_anchor_y"}) {
            auto *field = inspector->findChild<ScrubField *>(QString::fromLatin1(pair));
            QVERIFY(field && field->isVisible());
            QVERIFY(field->showsWhole());
            InspectorRow *row = rowOf(field);
            QCOMPARE(field->mapTo(row, QPoint(field->width(), 0)).x() + 8, row->keyframeButton()->x());
        }
        // The group's reset shows only under the pointer; its fold chevron is ink3.
        QVERIFY(transform->resetButton()->isHidden());
        QCOMPARE(transform->chevron()->inkColor(), tokens::color(tokens::ink3));
        QVERIFY(inspector->scrollArea()->verticalScrollBarPolicy() == Qt::ScrollBarAlwaysOff);
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        for (const auto kind : {InspectorColumn::Kind::Video, InspectorColumn::Kind::Audio, InspectorColumn::Kind::Info}) {
            column->setKind(kind);
            QCoreApplication::processEvents();
            const QImage shot = column->grab().toImage();
            QCOMPARE(shot.pixelColor(QPoint(column->width() / 2, 2) * shot.devicePixelRatio()).rgb(), tokens::color(tokens::chrome).rgb());
            if (!shots.isEmpty()) QVERIFY(shot.save(QDir(shots).filePath(QStringLiteral("inspector-%1.png").arg(InspectorColumn::kindId(kind)))));
        }
        window.m_shell->setTray(StageShell::Tray::Finish);
        column->setKind(InspectorColumn::Kind::Video);
        QCoreApplication::processEvents();
        if (!shots.isEmpty()) QVERIFY(column->grab().toImage().save(QDir(shots).filePath(QStringLiteral("inspector-finish.png"))));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::InspectorTest)

#include "inspector_test.moc"
