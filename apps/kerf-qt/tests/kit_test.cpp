// Tests for the shared UI kit: the icon set and its SVG reader (ui/icons),
// the flat components (ui/components) and the scope maths (ui/scope_math).

#include "components.h"
#include "design_tokens.h"
#include "icons.h"
#include "test_registry.h"

#include <QAction>
#include <QApplication>
#include <QCoreApplication>
#include <QCursor>
#include <QEnterEvent>
#include <QHelpEvent>
#include <QHoverEvent>
#include <QImage>
#include <QMenu>
#include <QMouseEvent>
#include <QObject>
#include <QPainter>
#include <QPainterPath>
#include <QScrollBar>
#include <QSignalSpy>
#include <QTest>
#include <QToolTip>
#include <QVBoxLayout>
#include <QWidget>

#include <cmath>
#include <memory>
#include <thread>

namespace kerf {

namespace {

using namespace kerf::tokens;

bool near(QPointF a, QPointF b, qreal tolerance = 0.02) {
    return std::abs(a.x() - b.x()) <= tolerance && std::abs(a.y() - b.y()) <= tolerance;
}

QString pointText(QPointF p) { return QStringLiteral("(%1, %2)").arg(p.x()).arg(p.y()); }

// The colour at (x, y) of `name` painted at `px` in `color` on a transparent
// 16 x 16 (or larger) image.
QColor iconPixel(const QString &name, QColor color, QPointF at, qreal px = 16) {
    const int side = int(px) * 4;
    QImage image(side, side, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    {
        QPainter p(&image);
        p.scale(4, 4);
        icons::paint(p, name, QRectF(0, 0, px, px), color, px);
    }
    return image.pixelColor((at * 4).toPoint());
}

void enter(QWidget *widget) {
    QEnterEvent event(QPointF(2, 2), QPointF(2, 2), QPointF(widget->mapToGlobal(QPoint(2, 2))));
    QCoreApplication::sendEvent(widget, &event);
}

void leave(QWidget *widget) {
    QEvent event(QEvent::Leave);
    QCoreApplication::sendEvent(widget, &event);
}

// The pointer moving within `widget` to `at`, as a widget with WA_Hover
// (and each of its WA_Hover parents) is told.
void hoverMove(QWidget *widget, QPoint at) {
    QHoverEvent event(QEvent::HoverMove, QPointF(at), QPointF(widget->mapToGlobal(at)), QPointF(at));
    QCoreApplication::sendEvent(widget, &event);
}

// Somewhere no test window reaches, so a real pointer never lands on one.
const QPoint kParked(-10000, -10000);

QImage solid(QSize size, QRgb rgb) {
    QImage image(size, QImage::Format_RGBA8888);
    image.fill(QColor::fromRgb(rgb));
    return image;
}

} // namespace

class KitTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with motion off, so nothing animates unless it asks,
    // and with the pointer away from every window it opens.
    void init() {
        ui::setReducedMotionOverride(1);
        QCursor::setPos(kParked);
        QCoreApplication::processEvents();
    }
    void cleanup() {
        QCursor::setPos(kParked);
        ui::setReducedMotionOverride(-1);
        ui::TooltipManager::instance()->setClock({});
        ui::TooltipManager::instance()->hide();
    }

    // ------------------------------------------------------------- icons

    // The table is spec 16 without the device preview's glyphs, the panes' and
    // folders' glyphs, and the two drawn outside the 16 set.
    void kitIconTableIsTheDesignsSet() {
        const QStringList names = icons::names();
        QCOMPARE(names.size(), 80);
        QCOMPARE(names.first(), QStringLiteral("g-169"));
        for (const char *name : {"g-169", "g-916", "g-comments", "g-settings", "g-chev", "g-kerf", "g-gauge", "g-cdr",
                                 "playhead-head", "checkbox-check"})
            QVERIFY2(icons::has(QString::fromLatin1(name)), name);
        QVERIFY(!icons::has(QStringLiteral("g-isl")));
        QVERIFY(!icons::has(QStringLiteral("g-punch")));
        QVERIFY(!icons::has(QStringLiteral("g-phone")));
        QVERIFY(!icons::has(QStringLiteral("rail-chevron")));
        QCOMPARE(icons::viewBox(QStringLiteral("g-play")), QSizeF(16, 16));
        QCOMPARE(icons::viewBox(QStringLiteral("g-settings")), QSizeF(24, 24));
        QCOMPARE(icons::viewBox(QStringLiteral("playhead-head")), QSizeF(9, 12));
        QVERIFY(icons::path(QStringLiteral("no-such-icon")).isEmpty());
        QVERIFY(icons::viewBox(QStringLiteral("no-such-icon")).isEmpty());
    }

    // Every icon parses without a problem, and paints something, all of it
    // inside the rect it is given, at 16 and at 14 pt.
    void kitEveryIconParsesAndPaintsInsideItsRect() {
        QVERIFY2(icons::problems().isEmpty(), qPrintable(icons::problems().join(QStringLiteral("; "))));
        for (const QString &name : icons::names()) {
            QVERIFY2(!icons::path(name).isEmpty(), qPrintable(name));
            for (const qreal px : {16.0, 14.0}) {
                QImage image(64, 64, QImage::Format_ARGB32_Premultiplied);
                image.fill(Qt::transparent);
                const QRectF rect(20, 20, px, px);
                {
                    QPainter p(&image);
                    icons::paint(p, name, rect, Qt::white, px);
                }
                int inside = 0;
                for (int y = 0; y < image.height(); ++y)
                    for (int x = 0; x < image.width(); ++x) {
                        if (qAlpha(image.pixel(x, y)) == 0) continue;
                        QVERIFY2(rect.contains(QPointF(x + 0.5, y + 0.5)),
                                 qPrintable(QStringLiteral("%1 at %2 paints (%3, %4)").arg(name).arg(px).arg(x).arg(y)));
                        ++inside;
                    }
                QVERIFY2(inside > 4, qPrintable(name));
                QVERIFY2(QRectF(0, 0, 16, 16).united(QRectF(QPointF(), icons::viewBox(name)))
                             .contains(icons::strokedPath(name, px).boundingRect()),
                         qPrintable(name));
            }
        }
    }

    // Arcs convert from endpoint to centre form: the gauge's half circle, the
    // comment bubble's corner, radii scaled up to the chord, compact flags and
    // a rotated ellipse all pass through the points SVG puts them through.
    void kitArcParsingMatchesKnownPoints() {
        QString error;
        // g-gauge: a half circle of radius 5.5 about (8, 11), over the top.
        const QPainterPath gauge = icons::svg::parsePathData(u"M2.5 11a5.5 5.5 0 1 1 11 0", &error);
        QVERIFY2(error.isEmpty(), qPrintable(error));
        QVERIFY(near(gauge.currentPosition(), QPointF(13.5, 11), 1e-9));
        QVERIFY2(near(gauge.pointAtPercent(0.5), QPointF(8, 5.5)), qPrintable(pointText(gauge.pointAtPercent(0.5))));
        for (int i = 0; i <= 20; ++i) {
            const QPointF p = gauge.pointAtPercent(i / 20.0);
            QVERIFY(std::abs(std::hypot(p.x() - 8, p.y() - 11) - 5.5) < 0.01);
            QVERIFY(p.y() <= 11 + 1e-9);
        }
        // g-comments' corner: a quarter circle about (3.5, 3.5).
        const QPainterPath corner = icons::svg::parsePathData(u"M2.5 3.5a1 1 0 0 1 1-1", &error);
        const qreal d = std::sqrt(0.5);
        QVERIFY2(near(corner.pointAtPercent(0.5), QPointF(3.5 - d, 3.5 - d), 0.005),
                 qPrintable(pointText(corner.pointAtPercent(0.5))));
        // Radii too small for the chord grow to fit it: radius 5 about (5, 0);
        // sweep 0 runs the other way round, through (5, 5).
        const QPainterPath grown = icons::svg::parsePathData(u"M0 0A1 1 0 0 0 10 0", &error);
        QVERIFY2(near(grown.pointAtPercent(0.5), QPointF(5, 5)), qPrintable(pointText(grown.pointAtPercent(0.5))));
        // Flags run into the numbers that follow them.
        const QPainterPath compact = icons::svg::parsePathData(u"M0 0a5 5 0 1010 0", &error);
        QVERIFY2(error.isEmpty(), qPrintable(error));
        QVERIFY(near(compact.currentPosition(), QPointF(10, 0), 1e-9));
        QVERIFY2(near(compact.pointAtPercent(0.5), QPointF(5, 5)), qPrintable(pointText(compact.pointAtPercent(0.5))));
        // A 2 x 1 ellipse turned 90 degrees: its long axis is vertical.
        const QPainterPath turned = icons::svg::parsePathData(u"M0 0A2 1 90 0 1 0 4", &error);
        const QPointF mid = turned.pointAtPercent(0.5);
        QVERIFY2(near(QPointF(std::abs(mid.x()), mid.y()), QPointF(1, 2)), qPrintable(pointText(mid)));
        // Equal endpoints draw nothing; a zero radius is a straight line.
        QCOMPARE(icons::svg::parsePathData(u"M1 1a2 2 0 0 1 0 0").elementCount(), 1);
        const QPainterPath flat = icons::svg::parsePathData(u"M0 0a0 2 0 0 1 4 0");
        QCOMPARE(flat.elementCount(), 2);
        QVERIFY(flat.elementAt(1).isLineTo());
    }

    // The rest of the path grammar: compact numbers, implicit repeats, the
    // smooth curve's reflected control, relative moves after a close, errors.
    void kitPathParserReadsTheGrammarTheIconsUse() {
        QString error;
        const QPainterPath head = icons::svg::parsePathData(u"M.5 .5h8v7L4.5 11.5.5 7.5z", &error);
        QVERIFY2(error.isEmpty(), qPrintable(error));
        const QPointF expected[] = {{0.5, 0.5}, {8.5, 0.5}, {8.5, 7.5}, {4.5, 11.5}, {0.5, 7.5}};
        for (int i = 0; i < 5; ++i) QCOMPARE(QPointF(head.elementAt(i)), expected[i]);
        // A moveto's extra pairs are linetos.
        const QPainterPath check = icons::svg::parsePathData(u"M3.5 8.5 6.5 11.5 12.5 4.5");
        QCOMPARE(check.elementCount(), 3);
        QVERIFY(check.elementAt(2).isLineTo());
        // S reflects the previous control point about the current point.
        const QPainterPath smooth = icons::svg::parsePathData(u"M0 0C0 1 1 1 1 0S2-1 2 0");
        QCOMPARE(QPointF(smooth.elementAt(4)), QPointF(1, -1));
        QCOMPARE(QPointF(smooth.elementAt(6)), QPointF(2, 0));
        // After z the current point is the subpath's start again.
        const QPainterPath closed = icons::svg::parsePathData(u"m1 1h2v2h-2zl1 1");
        QCOMPARE(closed.currentPosition(), QPointF(2, 2));
        // Exponents, and a sign that starts the next number.
        const QPainterPath exponent = icons::svg::parsePathData(u"M1e1-2E-1L0 0", &error);
        QVERIFY2(error.isEmpty(), qPrintable(error));
        QCOMPARE(QPointF(exponent.elementAt(0)), QPointF(10, -0.2));
        // Errors name where they are and keep what was read before them.
        icons::svg::parsePathData(u"M1 2L3", &error);
        QVERIFY(!error.isEmpty());
        icons::svg::parsePathData(u"M1 2z3 4", &error);
        QVERIFY(!error.isEmpty());
        const QPainterPath partial = icons::svg::parsePathData(u"M1 2L3 4X", &error);
        QVERIFY(error.contains(QLatin1Char('X')));
        QCOMPARE(partial.currentPosition(), QPointF(3, 4));
    }

    // Primitives start where SVG starts them, so dashes line up.
    void kitRectAndEllipseStartWhereSvgStartsThem() {
        const QPainterPath rect = icons::svg::rectPath(2.5, 3.5, 11, 9, 1);
        QCOMPARE(QPointF(rect.elementAt(0)), QPointF(3.5, 3.5));
        QCOMPARE(rect.boundingRect(), QRectF(2.5, 3.5, 11, 9));
        const QPainterPath ellipse = icons::svg::ellipsePath(8, 8, 5.5, 4.5);
        QCOMPARE(QPointF(ellipse.elementAt(0)), QPointF(13.5, 8));
        // Clockwise on screen: a quarter of the way round is the bottom.
        QVERIFY2(near(ellipse.pointAtPercent(0.25), QPointF(8, 12.5), 0.05),
                 qPrintable(pointText(ellipse.pointAtPercent(0.25))));
        // rx is clamped to half the side.
        QCOMPARE(icons::svg::rectPath(0, 0, 4, 4, 9).boundingRect(), QRectF(0, 0, 4, 4));
    }

    // Strokes are 1.5 in the viewBox, 1.7143 at 14 pt (1.5 rendered), and
    // markup that names its own width keeps it.
    void kitIconStrokeFollowsTheDesign() {
        QCOMPARE(icons::defaultStrokeWidth(16), 1.5);
        QCOMPARE(icons::defaultStrokeWidth(14), 1.7143);
        QCOMPARE(icons::defaultStrokeWidth(12), 1.5);
        // g-zout is one 9-unit line with round caps.
        const QRectF at16 = icons::strokedPath(QStringLiteral("g-zout"), 16).boundingRect();
        QVERIFY(std::abs(at16.width() - 10.5) < 0.05 && std::abs(at16.height() - 1.5) < 0.05);
        const QRectF at14 = icons::strokedPath(QStringLiteral("g-zout"), 14).boundingRect();
        QVERIFY(std::abs(at14.height() - 1.7143) < 0.05);
        // The gear's circle (r 3) at 2.25: 3 + 1.125 out from the centre.
        const QRectF gear = icons::strokedPath(QStringLiteral("g-settings"), 14).boundingRect();
        QVERIFY(gear.left() < 2 && gear.left() > 0.5);
        // The playhead head is drawn at its own 9 x 12.
        QCOMPARE(icons::iconBox(QStringLiteral("playhead-head"), QRectF(0, 0, 12, 12), 12), QRectF(1.5, 0, 9, 12));
    }

    // currentColor takes the colour asked for; fixed colours in the markup
    // stay; filled shapes fill and dashed shapes leave gaps.
    void kitIconsPaintInTheirColours() {
        const QColor ink2 = color(tokens::ink2);
        QCOMPARE(iconPixel(QStringLiteral("g-play"), ink2, QPointF(7.5, 8)), ink2);
        QCOMPARE(iconPixel(QStringLiteral("playhead-head"), ink2, QPointF(4.5, 4), 12), color(playhead));
        // g-ringdone: a filled disc with the check cut in chrome.
        QCOMPARE(iconPixel(QStringLiteral("g-ringdone"), ink2, QPointF(8, 3)), ink2);
        QCOMPARE(iconPixel(QStringLiteral("g-ringdone"), ink2, QPointF(7.1, 10)), color(chrome));
        // g-dim's evenodd fill at 35% leaves the inner window empty.
        const QColor dimmed = iconPixel(QStringLiteral("g-dim"), Qt::white, QPointF(3, 8));
        QVERIFY(dimmed.alpha() > 70 && dimmed.alpha() < 110);
        QCOMPARE(iconPixel(QStringLiteral("g-dim"), Qt::white, QPointF(8, 8)).alpha(), 0);
        // g-mrect is dashed (2 1.5): near the outside of its top edge the
        // round caps of neighbouring dashes leave notches, which a solid
        // outline (g-169's) does not have.
        const QPainterPath dashed = icons::strokedPath(QStringLiteral("g-mrect"));
        const QPainterPath solidEdge = icons::strokedPath(QStringLiteral("g-169"));
        int covered = 0, notches = 0;
        for (qreal x = 4; x < 12; x += 0.05) {
            (dashed.contains(QPointF(x, 3.5 - 0.65)) ? covered : notches)++;
            QVERIFY(solidEdge.contains(QPointF(x, 4 - 0.65)));
        }
        QVERIFY2(covered > 0 && notches > 0, qPrintable(QStringLiteral("%1/%2").arg(covered).arg(notches)));
        // The checkbox check is #a8a8a8 whatever colour is asked for.
        QCOMPARE(iconPixel(QStringLiteral("checkbox-check"), Qt::red, QPointF(6.5, 11.4), 16), ink2);
    }

    // QIcon carries every state colour at 1x and 2x.
    void kitIconStatesCarryTheirColours() {
        const QIcon icon = icons::icon(QStringLiteral("g-play"));
        auto centre = [&](QIcon::Mode mode, QIcon::State state, qreal dpr) {
            const QPixmap pm = icon.pixmap(QSize(16, 16), dpr, mode, state);
            return pm.toImage().pixelColor(QPointF(7.5 * pm.devicePixelRatio(), 8 * pm.devicePixelRatio()).toPoint());
        };
        for (const qreal dpr : {1.0, 2.0}) {
            QCOMPARE(centre(QIcon::Normal, QIcon::Off, dpr), color(ink2));
            QCOMPARE(centre(QIcon::Active, QIcon::Off, dpr), color(ink));
            QCOMPARE(centre(QIcon::Normal, QIcon::On, dpr), color(ink));
            QCOMPARE(centre(QIcon::Disabled, QIcon::Off, dpr), color(dis));
        }
        QCOMPARE(icon.pixmap(QSize(16, 16), 2.0).size(), QSize(32, 32));
        QVERIFY(icons::icon(QStringLiteral("nothing")).isNull());
    }

    // ------------------------------------------------------------- components

    // Icon buttons are 24, 28 or 20 square, with the icon in ink2, ink when
    // on, dis when disabled, and the on fill with its inset line.
    void kitIconButtonStates() {
        ui::IconButton button(QStringLiteral("g-import"));
        QCOMPARE(button.size(), QSize(24, 24));
        QCOMPARE(button.iconPx(), 16.0);
        QCOMPARE(button.iconColor(), color(ink2));
        button.setOn(true);
        QVERIFY(button.isOn());
        QCOMPARE(button.iconColor(), color(ink));
        QImage image(button.size(), QImage::Format_ARGB32_Premultiplied);
        image.fill(Qt::black);
        button.render(&image);
        QCOMPARE(image.pixelColor(12, 3), color(on));
        button.setOn(false);
        button.setEnabled(false);
        QCOMPARE(button.iconColor(), color(dis));
        button.setButtonSize(ui::IconButton::Size::S28);
        QCOMPARE(button.size(), QSize(28, 28));
        ui::IconButton small(QStringLiteral("g-chevl"), ui::IconButton::Size::S20);
        QCOMPARE(small.size(), QSize(20, 20));
        QCOMPARE(small.iconPx(), 14.0);
    }

    // The segmented control reserves each label's width at weight 500, so
    // turning segments on never moves anything; the fill covers the current
    // segment; the tail is 20 wide; the toolbar track is 28 tall.
    void kitSegmentedControlDoesNotShiftWhenTheWeightChanges() {
        ui::SegmentedControl seg;
        const QStringList labels{QStringLiteral("Edit"), QStringLiteral("Review"), QStringLiteral("Finish"),
                                 QStringLiteral("Deliver")};
        for (const QString &label : labels) seg.addSegment(label, label + QStringLiteral(" workspace"));
        seg.setMenuTail(true, QStringLiteral("Workspaces"));
        seg.setTrackHeight(28);
        seg.show();
        QVERIFY(QTest::qWaitForWindowExposed(&seg));
        const QSize size = seg.size();
        QList<QRectF> rects;
        for (int i = 0; i < seg.count(); ++i) {
            rects << seg.segmentRect(i);
            QCOMPARE(int(rects[i].width()), 16 + ui::snapWidth(controlStrong, labels[i]));
            QVERIFY(rects[i].width() >= 16 + ui::snapWidth(control, labels[i]));
            QCOMPARE(rects[i].height(), 24.0);
            QCOMPARE(rects[i].top(), 2.0);
        }
        QCOMPARE(rects.first().left(), 2.0);
        QCOMPARE(seg.menuTail()->width(), 20);
        QCOMPARE(size.height(), 28);
        QCOMPARE(size.width(), int(rects.last().right()) + 20 + 2);
        for (int i : {1, 3, 0, 2}) {
            seg.setCurrentIndex(i);
            QCOMPARE(seg.currentIndex(), i);
            QCOMPARE(seg.size(), size);
            for (int j = 0; j < seg.count(); ++j) QCOMPARE(seg.segmentRect(j), rects[j]);
            QCOMPARE(seg.fillRect(), rects[i]);
        }
        // Icon segments are 24 wide, or as set.
        ui::SegmentedControl formats;
        formats.addIconSegment(QStringLiteral("g-169"));
        formats.addIconSegment(QStringLiteral("g-916"));
        QCOMPARE(formats.segmentRect(1), QRectF(26, 2, 24, 20));
        formats.setIconSegmentWidth(28);
        QCOMPARE(formats.size(), QSize(2 + 56 + 2, 24));
        // Clicking a segment selects it and reports it.
        QSignalSpy activated(&seg, &ui::SegmentedControl::activated);
        QTest::mouseClick(seg.segment(1), Qt::LeftButton);
        QCOMPARE(seg.currentIndex(), 1);
        QCOMPARE(activated.size(), 1);
        QSignalSpy menu(&seg, &ui::SegmentedControl::menuRequested);
        QTest::mouseClick(seg.menuTail(), Qt::LeftButton);
        QCOMPARE(menu.size(), 1);
    }

    // W3 kit additions: none chosen (-1), one width for text segments, a
    // count after a label in ink3, the ghost button's meta look, and a
    // menu's own least card width.
    void kitSegmentsCountsAndMenusTakeTheW3Options() {
        ui::SegmentedControl seg;
        for (const char *label : {"S", "M", "L"}) seg.addSegment(QString::fromLatin1(label));
        seg.setCurrentIndex(1);
        QSignalSpy changed(&seg, &ui::SegmentedControl::currentChanged);
        seg.setCurrentIndex(-1);
        QCOMPARE(seg.currentIndex(), -1);
        QVERIFY(seg.fillRect().isEmpty());
        QCOMPARE(changed.size(), 1);
        QCOMPARE(changed.first().first().toInt(), -1);
        seg.setCurrentIndex(-1);
        QCOMPARE(changed.size(), 1);
        seg.setCurrentIndex(-2);
        QCOMPARE(seg.currentIndex(), -1);
        // From none, the fill appears where it lands (nothing to slide from).
        seg.setCurrentIndex(2);
        QCOMPARE(seg.fillRect(), seg.segmentRect(2));
        seg.setTextSegmentWidth(32);
        QCOMPARE(seg.textSegmentWidth(), 32);
        for (int i = 0; i < 3; ++i) QCOMPARE(seg.segmentRect(i), QRectF(2 + 32 * i, 2, 32, 20));
        QCOMPARE(seg.size(), QSize(2 + 96 + 2, 24));
        QCOMPARE(seg.fillRect(), seg.segmentRect(2));
        seg.setTextSegmentWidth(0);
        QCOMPARE(int(seg.segmentRect(0).width()), 16 + ui::snapWidth(controlStrong, QStringLiteral("S")));

        ui::SegmentedControl tabs;
        tabs.addSegment(QStringLiteral("Media"));
        tabs.addSegment(QStringLiteral("Index"));
        tabs.setSegmentCount(0, 12);
        QCOMPARE(tabs.segmentCount(0), 12);
        QCOMPARE(tabs.segmentCount(1), -1);
        QCOMPARE(int(tabs.segmentRect(0).width()),
                 16 + ui::snapWidth(controlStrong, QStringLiteral("Media")) + ui::snapWidth(control, QStringLiteral(" 12")));
        tabs.setCurrentIndex(0);
        tabs.resize(tabs.sizeHint());
        // The count is drawn in ink3, the label in ink.
        QImage shot(tabs.size() * 2, QImage::Format_ARGB32_Premultiplied);
        shot.setDevicePixelRatio(2);
        shot.fill(Qt::transparent);
        tabs.segment(0)->render(&shot);
        bool ink3Seen = false;
        for (int y = 0; y < shot.height() && !ink3Seen; ++y)
            for (int x = 0; x < shot.width(); ++x) {
                const QColor c = shot.pixelColor(x, y);
                if (c.alpha() == 255 && std::abs(c.lightness() - color(ink3).lightness()) <= 4) {
                    ink3Seen = true;
                    break;
                }
            }
        QVERIFY(ink3Seen);
        tabs.setSegmentCount(0, -1);
        QCOMPARE(int(tabs.segmentRect(0).width()), 16 + ui::snapWidth(controlStrong, QStringLiteral("Media")));

        ui::GhostButton edit(QStringLiteral("Edit roles…"));
        edit.setMeta(true);
        QVERIFY(edit.isMeta());
        QCOMPARE(edit.height(), 20);
        QCOMPARE(edit.textColor(), color(ink3));
        QCOMPARE(edit.typeStyle().px, tokens::meta.px);
        edit.setEnabled(false);
        QCOMPARE(edit.textColor(), color(dis));
        edit.setEnabled(true);
        edit.setMeta(false);
        QCOMPARE(edit.textColor(), color(ink2));

        QWidget window;
        window.resize(900, 600);
        auto *anchor = new ui::GhostButton(QStringLiteral("Workspaces"), &window);
        anchor->move(40, 40);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QMenu menu(&window);
        menu.addAction(QStringLiteral("Edit"));
        QCOMPARE(ui::menuMinimumCardWidth(&menu), 220);
        ui::setMenuMinimumCardWidth(&menu, 280);
        QCOMPARE(ui::menuMinimumCardWidth(&menu), 280);
        ui::popupMenu(&menu, anchor);
        QVERIFY(QTest::qWaitForWindowExposed(&menu));
        const QMargins m = ui::shadowMargins();
        QCOMPARE(menu.width() - 2 * qMax(m.left(), m.right()), 280);
        menu.close();
        ui::setMenuMinimumCardWidth(&menu, 0);
        QCOMPARE(ui::menuMinimumCardWidth(&menu), 220);
    }

    // The fill slides over 200 ms when motion is on, and jumps when it is off.
    void kitSegmentedFillSlidesOnlyWithMotion() {
        ui::setReducedMotionOverride(0);
        ui::SegmentedControl seg;
        for (const char *label : {"Edit", "Review", "Finish"}) seg.addSegment(QString::fromLatin1(label));
        seg.setCurrentIndex(0);
        seg.show();
        QVERIFY(QTest::qWaitForWindowExposed(&seg));
        seg.setCurrentIndex(2);
        QVERIFY(seg.fillRect() != seg.segmentRect(2));
        QTRY_COMPARE_WITH_TIMEOUT(seg.fillRect(), seg.segmentRect(2), 1000);
        ui::setReducedMotionOverride(1);
        seg.setCurrentIndex(0);
        QCOMPARE(seg.fillRect(), seg.segmentRect(0));
        QCOMPARE(ui::motionDuration(motion::segment), 0);
        ui::setReducedMotionOverride(0);
        QCOMPARE(ui::motionDuration(motion::segment), 200);
    }

    // Buttons, ghost buttons, chips, check boxes, badges and avatars keep the
    // design's sizes and colours.
    void kitFlatControlsKeepTheirMetrics() {
        ui::Button ok(QStringLiteral("OK"), ui::Button::Kind::Primary);
        QCOMPARE(ok.size(), QSize(64, 24));
        QCOMPARE(ok.background(), color(accent));
        QCOMPARE(ok.textColor(), color(accentText));
        ui::Button insert(QStringLiteral("Insert a long label here"));
        QCOMPARE(insert.width(), 20 + ui::snapWidth(controlStrong, insert.text()));
        QCOMPARE(insert.textColor(), color(ink));
        insert.setEnabled(false);
        QCOMPARE(insert.textColor(), color(dis));

        ui::GhostButton fit(QStringLiteral("Fit"));
        fit.setChevron(true);
        QCOMPARE(fit.size(), QSize(8 + ui::snapWidth(control, QStringLiteral("Fit")) + 4 + 14 + 8, 24));
        QCOMPARE(fit.textColor(), color(ink2));
        ui::GhostButton seq(QStringLiteral("1920×1080 · 25p"));
        seq.setMono(true);
        seq.setControlHeight(28);
        QCOMPARE(seq.size(), QSize(16 + ui::snapWidth(monoSmall, seq.text()), 28));

        ui::Chip chip(QStringLiteral("Open"));
        chip.setCount(4);
        const QSize chipSize = chip.size();
        QCOMPARE(chipSize.height(), 24);
        chip.setChecked(true);
        QCOMPARE(chip.size(), chipSize);

        ui::CheckBox box;
        QCOMPARE(box.size(), QSize(14, 14));
        QVERIFY(box.isCheckable());
        box.setChecked(true);
        QImage image(box.size() * 4, QImage::Format_ARGB32_Premultiplied);
        image.setDevicePixelRatio(4);
        image.fill(Qt::black);
        box.render(&image);
        QCOMPARE(image.pixelColor(4 * 2, 4 * 12), color(on));

        ui::Badge badge(QStringLiteral("4"));
        QCOMPARE(badge.badgeRect().size(), QSizeF(16, 16));
        QCOMPARE(badge.size(), QSize(20, 20));
        badge.setText(QStringLiteral("128"));
        QVERIFY(badge.badgeRect().width() > 16);

        QCOMPARE(ui::Avatar::initialsFor(QStringLiteral("Mina Park")), QStringLiteral("MP"));
        QCOMPARE(ui::Avatar::initialsFor(QStringLiteral("Claude Code")), QStringLiteral("CC"));
        QCOMPARE(ui::Avatar::initialsFor(QStringLiteral("Cursor")), QStringLiteral("Cu"));
        QCOMPARE(ui::Avatar::kindFor(QStringLiteral("person")), ui::Avatar::Kind::Person);
        QCOMPARE(ui::Avatar::kindFor(QStringLiteral("mcp")), ui::Avatar::Kind::Agent);
        QCOMPARE(ui::Avatar::kindFor(QStringLiteral("kerf")), ui::Avatar::Kind::Kerf);
        QCOMPARE(ui::Avatar::kindFor(QStringLiteral("qc")), ui::Avatar::Kind::Qc);
        ui::Avatar person(ui::Avatar::Kind::Person, QStringLiteral("MP"));
        ui::Avatar agent(ui::Avatar::Kind::Agent, QStringLiteral("Cu"));
        QCOMPARE(person.size(), QSize(24, 24));
        QVERIFY(person.isRound());
        QVERIFY(!agent.isRound());
        // A person's corner is outside the circle; an agent's square fills it.
        QImage corner(24, 24, QImage::Format_ARGB32_Premultiplied);
        corner.fill(Qt::black);
        person.render(&corner, QPoint(), QRegion(), QWidget::DrawChildren);
        QCOMPARE(corner.pixelColor(1, 1), QColor(Qt::black));
        corner.fill(Qt::black);
        agent.render(&corner, QPoint(), QRegion(), QWidget::DrawChildren);
        QCOMPARE(corner.pixelColor(4, 4), color(on));

        ui::Separator separator(8);
        QCOMPARE(separator.size(), QSize(17, 16));
        ui::Dot dot(color(pos));
        QCOMPARE(dot.size(), QSize(6, 6));
        ui::ThinProgress progress;
        progress.resize(120, 4);
        progress.setValue(0.62);
        QCOMPARE(progress.fillRect().width(), 120 * 0.62);
        QCOMPARE(progress.height(), 4);
    }

    // The slider's knob sits on the value inside 5 pt insets; a bipolar
    // slider fills from its centre and shows the centre tick.
    void kitSliderPlacesItsKnobOnTheValue() {
        ui::Slider slider;
        slider.resize(110, 16);
        slider.setRange(0, 100);
        slider.setValue(42);
        QCOMPARE(slider.trackRect(), QRectF(5, 7, 100, 2));
        QCOMPARE(slider.knobCentre(), QPointF(47, 8));
        QCOMPARE(slider.fillRect(), QRectF(5, 7, 42, 2));
        slider.setValue(500);
        QCOMPARE(slider.value(), 100.0);
        slider.setBipolar(true);
        slider.setRange(-100, 100);
        slider.setValue(-50);
        QCOMPARE(slider.fillRect(), QRectF(30, 7, 25, 2));
        // The user drags and steps it.
        QSignalSpy moved(&slider, &ui::Slider::sliderMoved);
        QTest::mouseClick(&slider, Qt::LeftButton, {}, QPoint(105, 8));
        QCOMPARE(slider.value(), 100.0);
        QCOMPARE(moved.size(), 1);
        slider.setSingleStep(10);
        QTest::keyClick(&slider, Qt::Key_Left);
        QCOMPARE(slider.value(), 90.0);
    }

    // The tip waits 400 ms, then shows name and shortcut; it is instant
    // while another shows or within 300 ms of one hiding; a press hides it;
    // Qt's own tooltip never shows for a widget with a kit tip.
    void kitTooltipTimingFollowsTheDesign() {
        qint64 now = 1000;
        auto *manager = ui::TooltipManager::instance();
        manager->setClock([&now] { return now; });
        QWidget window;
        window.resize(1600, 1000);
        auto *a = new ui::IconButton(QStringLiteral("g-import"), ui::IconButton::Size::S28, &window);
        auto *b = new ui::IconButton(QStringLiteral("g-undo"), ui::IconButton::Size::S28, &window);
        a->move(100, 6);
        b->move(132, 6);
        a->setTip(QStringLiteral("Import media"), QStringLiteral("⌘I"));
        b->setTip(QStringLiteral("Undo"), QStringLiteral("⌘Z"));
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));

        enter(a);
        QVERIFY(!manager->isShowing());
        now += 399;
        manager->tick();
        QVERIFY(!manager->isShowing());
        now += 1;
        manager->tick();
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->shownName(), QStringLiteral("Import media"));
        QCOMPARE(manager->shownShortcut(), QStringLiteral("⌘I"));
        // Moving to a neighbour while a tip shows is instant.
        enter(b);
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->shownName(), QStringLiteral("Undo"));
        // Within 300 ms of hiding, the next is instant too ...
        leave(b);
        QVERIFY(!manager->isShowing());
        now += 299;
        enter(a);
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->shownName(), QStringLiteral("Import media"));
        // ... but after that it waits again.
        leave(a);
        now += 301;
        enter(b);
        QVERIFY(!manager->isShowing());
        now += 400;
        manager->tick();
        QVERIFY(manager->isShowing());
        // A press hides it.
        QTest::mousePress(b, Qt::LeftButton);
        QVERIFY(!manager->isShowing());
        QTest::mouseRelease(b, Qt::LeftButton);
        // Qt's tooltip event is taken.
        QHelpEvent help(QEvent::ToolTip, QPoint(2, 2), b->mapToGlobal(QPoint(2, 2)));
        QVERIFY(QCoreApplication::sendEvent(b, &help));
        // A tip with no shortcut shows its name alone; an empty name removes it.
        ui::setTip(a, QStringLiteral("Undo"), {});
        QCOMPARE(ui::tipShortcut(a), QString());
        ui::setTip(a, {});
        QCOMPARE(ui::tipName(a), QString());
    }

    // A disabled control shows its tip after the same 400 ms, so it can say
    // why it is disabled (plan decision 3).
    void kitTooltipShowsOnDisabledControls() {
        qint64 now = 1000;
        auto *manager = ui::TooltipManager::instance();
        manager->setClock([&now] { return now; });
        QWidget window;
        window.resize(800, 600);
        auto *button = new ui::IconButton(QStringLiteral("g-import"), ui::IconButton::Size::S28, &window);
        button->move(100, 6);
        button->setEnabled(false);
        button->setTip(QStringLiteral("Resolve 2 blocking notes to render"));
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QVERIFY(!button->isEnabled());

        enter(button);
        now += 399;
        manager->tick();
        QVERIFY(!manager->isShowing());
        now += 1;
        manager->tick();
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->target(), button);
        QCOMPARE(manager->shownName(), QStringLiteral("Resolve 2 blocking notes to render"));
        // Qt's tooltip event is taken on a disabled control too.
        QHelpEvent help(QEvent::ToolTip, QPoint(2, 2), button->mapToGlobal(QPoint(2, 2)));
        QToolTip::hideText();
        QCoreApplication::sendEvent(button, &help);
        QVERIFY(QToolTip::text().isEmpty());
    }

    // As the mockup re-checks closest('[data-tip]') on every mouseover:
    // back on a tipped parent from its tipped child, the parent's tip takes
    // over; after a press hides a tip, moving on the control brings it back;
    // and a child with its own Qt tooltip keeps it inside a tipped parent.
    void kitTooltipFollowsThePointerBetweenParentAndChild() {
        qint64 now = 1000;
        auto *manager = ui::TooltipManager::instance();
        manager->setClock([&now] { return now; });
        QWidget window;
        window.resize(800, 600);
        auto *panel = new QWidget(&window);
        panel->setGeometry(100, 100, 300, 120);
        panel->setAttribute(Qt::WA_Hover);
        ui::setTip(panel, QStringLiteral("Comments · 3 open"));
        auto *child = new ui::IconButton(QStringLiteral("g-undo"), ui::IconButton::Size::S24, panel);
        child->move(10, 10);
        child->setTip(QStringLiteral("Undo"), QStringLiteral("⌘Z"));
        auto *own = new QWidget(panel);
        own->setGeometry(100, 10, 40, 40);
        own->setToolTip(QStringLiteral("Qt's own"));
        auto *plain = new QWidget(panel);
        plain->setGeometry(160, 10, 40, 40);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));

        enter(panel);
        now += 400;
        manager->tick();
        QCOMPARE(manager->shownName(), QStringLiteral("Comments · 3 open"));
        enter(child);
        QCOMPARE(manager->shownName(), QStringLiteral("Undo"));
        // A bare QEvent::Enter (no point, as some code sends it) is the
        // widget's own; it is not read as a pointer event.
        {
            QEvent bare(QEvent::Enter);
            QCoreApplication::sendEvent(panel, &bare);
            QCOMPARE(manager->target(), panel);
            QEvent bareChild(QEvent::Enter);
            QCoreApplication::sendEvent(child, &bareChild);
            QCOMPARE(manager->shownName(), QStringLiteral("Undo"));
        }

        // Leaving the child for the parent's own area: the parent gets no
        // Enter, and its tip comes back at once.
        const QPoint onPanel(250, 90);
        QCursor::setPos(panel->mapToGlobal(onPanel));
        QCoreApplication::processEvents();
        leave(child);
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->target(), panel);
        QCOMPARE(manager->shownName(), QStringLiteral("Comments · 3 open"));

        // A move on the parent at the child's place is the child's (the
        // move reaches the parent too, but the child is under the pointer).
        hoverMove(panel, child->geometry().center());
        QCOMPARE(manager->shownName(), QStringLiteral("Undo"));
        hoverMove(panel, onPanel);
        QCOMPARE(manager->shownName(), QStringLiteral("Comments · 3 open"));

        // A press hides the tip; the next move, with the button up, brings
        // it back (instantly, within 300 ms of the hide).
        QTest::mousePress(panel, Qt::LeftButton, {}, onPanel);
        QVERIFY(!manager->isShowing());
        // Dragging with the button down does not.
        {
            const QPoint at = onPanel + QPoint(1, 0);
            QMouseEvent drag(QEvent::MouseMove, QPointF(at), QPointF(panel->mapToGlobal(at)), Qt::NoButton,
                             Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(panel, &drag);
        }
        QVERIFY(!manager->isShowing());
        QTest::mouseRelease(panel, Qt::LeftButton, {}, onPanel);
        hoverMove(panel, onPanel + QPoint(2, 0));
        QVERIFY(manager->isShowing());
        QCOMPARE(manager->target(), panel);
        // Long after a press, the same move waits the 400 ms again.
        QTest::mousePress(panel, Qt::LeftButton, {}, onPanel);
        QTest::mouseRelease(panel, Qt::LeftButton, {}, onPanel);
        now += 301;
        hoverMove(panel, onPanel);
        QVERIFY(!manager->isShowing());
        now += 400;
        manager->tick();
        QVERIFY(manager->isShowing());

        // Qt's tooltip: kept for the child with its own, taken for a child
        // that shows the parent's kit tip, and for the parent itself.
        QToolTip::hideText();
        QHelpEvent ownHelp(QEvent::ToolTip, QPoint(2, 2), own->mapToGlobal(QPoint(2, 2)));
        QCoreApplication::sendEvent(own, &ownHelp);
        QCOMPARE(QToolTip::text(), QStringLiteral("Qt's own"));
        QToolTip::hideText();
        QHelpEvent plainHelp(QEvent::ToolTip, QPoint(2, 2), plain->mapToGlobal(QPoint(2, 2)));
        // Taken: the event stays accepted (QWidget would ignore it, having no
        // tooltip of its own) and no Qt tooltip shows the panel's text.
        QCoreApplication::sendEvent(plain, &plainHelp);
        QVERIFY(plainHelp.isAccepted());
        panel->setToolTip(QStringLiteral("hidden by the kit tip"));
        QHelpEvent panelHelp(QEvent::ToolTip, onPanel, panel->mapToGlobal(onPanel));
        QCoreApplication::sendEvent(panel, &panelHelp);
        QVERIFY(QToolTip::text() != QStringLiteral("hidden by the kit tip"));
        QHelpEvent plainAgain(QEvent::ToolTip, QPoint(2, 2), plain->mapToGlobal(QPoint(2, 2)));
        QCoreApplication::sendEvent(plain, &plainAgain);
        QVERIFY(QToolTip::text() != QStringLiteral("hidden by the kit tip"));
        // Without the kit tip, the same event is Qt's again.
        ui::setTip(panel, {});
        panel->setToolTip(QStringLiteral("Qt's again"));
        QHelpEvent plainQt(QEvent::ToolTip, QPoint(2, 2), plain->mapToGlobal(QPoint(2, 2)));
        QCoreApplication::sendEvent(plain, &plainQt);
        QCOMPARE(QToolTip::text(), QStringLiteral("Qt's again"));
        QToolTip::hideText();
    }

    // Tips go 6 below the control, centred, or 6 above it when the control's
    // bottom is past 540 and its top past 100; x stays within 8 ... W - 8.
    void kitTooltipPlacementFollowsTheFiveFortyRule() {
        using M = ui::TooltipManager;
        const QSize window(1600, 1000), tip(80, 24);
        QCOMPARE(M::placement(QRectF(100, 100, 24, 24), tip, window), QRect(72, 130, 80, 24));
        QCOMPARE(M::placement(QRectF(100, 900, 24, 24), tip, window), QRect(72, 870, 80, 24));
        // A tall control whose top is above 100 keeps its tip below.
        QCOMPARE(M::placement(QRectF(100, 80, 24, 520), tip, window).top(), 606);
        // A bottom of exactly 540 is not past it.
        QCOMPARE(M::placement(QRectF(100, 516, 24, 24), tip, window).top(), 546);
        QCOMPARE(M::placement(QRectF(0, 100, 24, 24), tip, window).left(), 8);
        QCOMPARE(M::placement(QRectF(1590, 100, 10, 24), tip, window).left(), 1512);
    }

    // Menus and popovers open 4 below the anchor, flip above it past H - 32,
    // move left past W - 8, and open to the right when asked.
    void kitPopoverPlacementFlipsPastTheEdges() {
        const QRect bounds(0, 0, 1600, 1000);
        const QSize size(220, 200);
        QCOMPARE(ui::popoverPlacement(QRectF(100, 100, 24, 24), size, bounds), QRect(QPoint(100, 128), size));
        QCOMPARE(ui::popoverPlacement(QRectF(100, 800, 24, 24), size, bounds), QRect(QPoint(100, 596), size));
        // Ending exactly at H - 32 still fits below.
        QCOMPARE(ui::popoverPlacement(QRectF(100, 740, 24, 24), size, bounds).top(), 768);
        QCOMPARE(ui::popoverPlacement(QRectF(100, 741, 24, 24), size, bounds).top(), 537);
        // Never above the top margin when flipped.
        QCOMPARE(ui::popoverPlacement(QRectF(100, 150, 24, 24), QSize(220, 900), bounds).top(), 8);
        QCOMPARE(ui::popoverPlacement(QRectF(1500, 100, 24, 24), size, bounds).left(), 1304);
        QCOMPARE(ui::popoverPlacement(QRectF(100, 100, 24, 24), size, bounds, ui::Side::Right),
                 QRect(QPoint(132, 100), size));
        QCOMPARE(ui::popoverPlacement(QRectF(1500, 100, 24, 24), size, bounds, ui::Side::Right),
                 QRect(QPoint(1304, 128), size));

        // A popover opens at that place, its card inside the shadow margins.
        QWidget window;
        window.resize(800, 600);
        auto *anchor = new ui::GhostButton(QStringLiteral("Fit"), &window);
        anchor->move(100, 500);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::Popover popover(&window);
        auto *content = new QWidget;
        content->setFixedSize(212, 92);
        popover.setContent(content);
        popover.openAt(anchor);
        QVERIFY(popover.isVisible());
        const QRect card = popover.cardGeometry();
        QCOMPARE(card.size(), QSize(220, 100));
        QCOMPARE(card.topLeft(), window.mapToGlobal(QPoint(100, 500 - 100 - 4)));
        popover.close();
    }

    // The shadow halo is part of the popup window but looks like outside:
    // a press there closes the popover (emitting closed()) or the menu, as
    // a click outside does; a press on the card does not.
    void kitPopoverAndMenusCloseOnAPressInTheirShadow() {
        QWidget window;
        window.resize(800, 600);
        auto *anchor = new ui::GhostButton(QStringLiteral("Fit"), &window);
        anchor->move(100, 100);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const QMargins m = ui::shadowMargins();

        ui::Popover popover(&window);
        auto *content = new QWidget;
        content->setFixedSize(212, 92);
        popover.setContent(content);
        QSignalSpy closed(&popover, &ui::Popover::closed);
        popover.openAt(anchor);
        QVERIFY(popover.isVisible());
        // On the card's padding: it stays open.
        QTest::mousePress(&popover, Qt::LeftButton, {}, QPoint(m.left() + 2, m.top() + 2));
        QTest::mouseRelease(&popover, Qt::LeftButton, {}, QPoint(m.left() + 2, m.top() + 2));
        QVERIFY(popover.isVisible());
        QCOMPARE(closed.count(), 0);
        // In the halo, left of the card and below it: it closes.
        QTest::mousePress(&popover, Qt::LeftButton, {}, QPoint(m.left() / 2, popover.height() / 2));
        QTRY_VERIFY(!popover.isVisible());
        QCOMPARE(closed.count(), 1);
        popover.openAt(anchor);
        QVERIFY(popover.isVisible());
        QTest::mousePress(&popover, Qt::LeftButton, {}, QPoint(popover.width() / 2, popover.height() - 2));
        QTRY_VERIFY(!popover.isVisible());
        QCOMPARE(closed.count(), 2);

        QMenu menu(&window);
        QAction *first = menu.addAction(QStringLiteral("Laptop"));
        menu.addAction(QStringLiteral("Phone"));
        QSignalSpy triggered(&menu, &QMenu::triggered);
        const int h = qMax(m.left(), m.right()), v = qMax(m.top(), m.bottom());
        ui::popupMenu(&menu, anchor);
        QVERIFY(QTest::qWaitForWindowExposed(&menu));
        // A press on the card's padding, above the first row, keeps it open.
        // (No release: QMenu closes on a release far from where the pointer
        // was when it opened, and the test's pointer is parked far away.)
        const QPoint padding(h + 2, v + 2);
        QVERIFY(!menu.actionGeometry(first).contains(padding));
        QTest::mousePress(&menu, Qt::LeftButton, {}, padding);
        QVERIFY(menu.isVisible());
        // In the halo above the card, then below it on a second opening.
        QTest::mousePress(&menu, Qt::LeftButton, {}, QPoint(menu.width() / 2, 2));
        QTRY_VERIFY(!menu.isVisible());
        ui::popupMenu(&menu, anchor);
        QVERIFY(QTest::qWaitForWindowExposed(&menu));
        QTest::mousePress(&menu, Qt::LeftButton, {}, QPoint(h + 10, menu.height() - 4));
        QTRY_VERIFY(!menu.isVisible());
        QCOMPARE(triggered.count(), 0);
    }

    // A kit menu has 28 pt items, 9 pt separators, 28 pt headers, a check
    // column, keys on the right, and paints its own card.
    void kitStyledMenusFollowTheMenuSpec() {
        // The same menu, without an app style sheet and under one that styles
        // QMenu its own way: the kit's metrics and painting win both times.
        const QString appSheet = qApp->styleSheet();
        checkStyledMenu();
        if (QTest::currentTestFailed()) return;
        qApp->setStyleSheet(QStringLiteral("QMenu { padding: 2px; border: 3px solid red; background: red; }"
                                           " QMenu::item { padding: 9px 30px; } QMenu::item:selected { background: red; }"
                                           " QMenu::separator { height: 1px; margin: 6px; }"));
        checkStyledMenu();
        qApp->setStyleSheet(appSheet);
    }

    // Fades show only where content is cut: none at rest at the top, the
    // bottom one while more is below, the top one once a child is cut.
    void kitFadeScrollAreaFadesWhereContentIsCut() {
        ui::FadeScrollArea area;
        area.resize(300, 300);
        auto *content = new QWidget;
        auto *layout = new QVBoxLayout(content);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(0);
        for (int i = 0; i < 10; ++i) {
            auto *row = new QWidget;
            row->setFixedHeight(100);
            layout->addWidget(row);
        }
        area.setWidget(content);
        area.show();
        QVERIFY(QTest::qWaitForWindowExposed(&area));
        QCOMPARE(area.verticalScrollBarPolicy(), Qt::ScrollBarAlwaysOff);
        QTRY_VERIFY(area.bottomFadeShown());
        QVERIFY(!area.topFadeShown());
        area.verticalScrollBar()->setValue(50);
        QVERIFY(area.topFadeShown());
        area.verticalScrollBar()->setValue(100);
        QVERIFY(!area.topFadeShown()); // a row boundary: nothing is cut
        area.verticalScrollBar()->setValue(area.verticalScrollBar()->maximum());
        QVERIFY(!area.bottomFadeShown());
        // The fade paints the surface colour at the very top, and only there.
        area.verticalScrollBar()->setValue(150);
        const QImage image = area.grab().toImage();
        const qreal dpr = image.devicePixelRatio();
        const QColor top = image.pixelColor(QPoint(150, 0) * dpr);
        QVERIFY2(std::abs(top.red() - color(card).red()) <= 12, qPrintable(top.name()));
        QVERIFY(image.pixelColor(QPoint(150, 40) * dpr) != top);
    }

private:
    void checkStyledMenu() {
        QWidget window;
        window.resize(800, 600);
        auto *anchor = new ui::IconButton(QStringLiteral("g-more"), ui::IconButton::Size::S24, &window);
        anchor->move(40, 40);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QMenu menu(&window);
        QAction *header = ui::addMenuHeader(&menu, QStringLiteral("Device preview · 16:9"));
        QAction *laptop = menu.addAction(QStringLiteral("Laptop"));
        laptop->setCheckable(true);
        laptop->setChecked(true);
        ui::setMenuKeyIcon(laptop, QStringLiteral("g-laptop"));
        QAction *sep = menu.addSeparator();
        QAction *sync = menu.addAction(QStringLiteral("Sync now"));
        ui::setMenuKey(sync, QStringLiteral("⌘S"));
        QAction *peak = menu.addAction(QStringLiteral("True peak −0.4 dBTP"));
        ui::setMenuDot(peak, color(neg));
        ui::setMenuAccent(peak, QStringLiteral("Ask Kerf"));
        QAction *blocking = menu.addAction(QStringLiteral("2 blocking notes open"));
        ui::setMenuRole(blocking, ui::MenuRole::Warn);
        QCOMPARE(ui::menuRole(header), ui::MenuRole::Header);
        QCOMPARE(ui::menuRole(blocking), ui::MenuRole::Warn);
        QVERIFY(!header->isEnabled());
        ui::popupMenu(&menu, anchor);
        QVERIFY(QTest::qWaitForWindowExposed(&menu));
        QCOMPARE(menu.actionGeometry(laptop).height(), 28);
        QCOMPARE(menu.actionGeometry(sync).height(), 28);
        QCOMPARE(menu.actionGeometry(sep).height(), 9);
        QCOMPARE(menu.actionGeometry(header).height(), 28);
        QCOMPARE(menu.actionGeometry(laptop).top(), menu.actionGeometry(header).bottom() + 1);
        const QMargins shadow = ui::shadowMargins();
        const int h = qMax(shadow.left(), shadow.right()), v = qMax(shadow.top(), shadow.bottom());
        QVERIFY(menu.width() - 2 * h >= 220);
        // The widest row fits: 8 + check 14 + 8 + label + 16 + accent + 8, in 4 of padding.
        const int peakRow = 8 + 14 + 8 + ui::snapWidth(control, peak->text()) + 16 +
                            ui::snapWidth(control, QStringLiteral("Ask Kerf")) + 8;
        QVERIFY(menu.actionGeometry(peak).width() >= peakRow);
        // The card opens 4 below the anchor, left-aligned with it.
        QCOMPARE(menu.pos() + QPoint(h, v), window.mapToGlobal(QPoint(40, 40 + 24 + 4)));
        // Items sit 4 inside the card.
        QCOMPARE(menu.actionGeometry(header).topLeft(), QPoint(h + 4, v + 4));
        const QImage image = menu.grab().toImage();
        const qreal dpr = image.devicePixelRatio();
        QCOMPARE(image.pixelColor((QPointF(h + 2, v + 40) * dpr).toPoint()), color(card));
        QCOMPARE(qAlpha(image.pixel(1, 1)), 0);
        menu.setActiveAction(sync);
        const QImage hovered = menu.grab().toImage();
        const QRect syncRect = menu.actionGeometry(sync);
        QCOMPARE(hovered.pixelColor((QPointF(syncRect.left() + 3, syncRect.center().y()) * dpr).toPoint()),
                 color(raised));
        menu.close();
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::KitTest)

#include "kit_test.moc"
