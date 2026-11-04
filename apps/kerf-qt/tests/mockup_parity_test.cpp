// The whole editor against the approved mockup (03-review-thread.html) at its
// 1600 x 1000 canvas: every workspace (review, edit, finish, deliver) in
// every format (16:9, 9:16, 1:1, 4:5), its comments panel open and closed.
// Each state's regions are checked against mockup-spec 0.1, its stage
// against the 3.1 [OWNER] table (with the W3 phone scrub: it ends 12 left of
// the rail), and its device against the exact MacBook Pro 14" and iPhone 16
// Pro sizes of spec 3.2 to within half a point. The editor is fed the seed
// (comments, proposals, a check, loudness and a render queue) as kerfd sends
// it, through handleEvent.
//
// With KERF_SCREENSHOT_DIR set, every state is saved under the mockup's
// query state: <ws>-<fmt>.png for the workspace as it opens (?ws=edit&fmt=916
// is edit-916.png), and <ws>-<fmt>-panel0.png / -panel1.png for the panel
// toggled (?panel=0).

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "components.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "device_frame.h"
#include "main_window.h"
#include "program_strip.h"
#include "source_monitor_panel.h"
#include "scrub_strip.h"
#include "stage_rail.h"
#include "test_footage.h"
#include "ui/editor_bus.h"
#include "ui/inspector_column.h"
#include "ui/review_model.h"
#include "ui/wiring/review_wiring.h"
#include "ui/wiring/stage_wiring.h"
#include "viewer_widget.h"
#include "workspace.h"

#include <DockAreaWidget.h>
#include <DockWidget.h>

#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QCoreApplication>
#include <QDir>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSettings>
#include <QStatusBar>
#include <QTest>
#include <QToolBar>

#include <cmath>

namespace kerf {

using namespace test;

namespace {

const QString kYou = QStringLiteral("ana@kerf.test");

QJsonObject person(const char *name, const QString &email, const char *role = nullptr) {
    QJsonObject author{{"kind", "person"}, {"name", name}, {"id", email}};
    if (role) author.insert("role", role);
    return author;
}

QJsonObject note(const char *id, const QJsonValue &parent, const QJsonObject &author, const char *created, const char *body,
                 std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record{{"id", id},       {"parent", parent}, {"author", author}, {"body", body},     {"tags", QJsonArray{}},
                       {"blocking", false}, {"resolved", false}, {"created", created}, {"revision", 22}};
    for (const auto &[key, value] : extra) record.insert(key, value);
    return record;
}

// The mockup's seed threads (mockup-spec 4.9), as kerfd's review log folds them.
QJsonObject seedComments() {
    const QJsonObject mina = person("Mina Park", QStringLiteral("mina@kerf.test"), "director");
    const QJsonObject tom = person("Tom Lind", QStringLiteral("tom@kerf.test"), "colourist");
    const QJsonObject you = person("Ana Ruiz", kYou);
    const QJsonObject kerf{{"kind", "kerf"}, {"name", "Kerf"}};
    const QJsonObject qc{{"kind", "qc"}, {"name", "QC"}, {"role", "loudness"}};
    const QJsonObject claude{{"kind", "mcp"}, {"name", "Claude Code"}, {"id", "claude-code"}, {"role", "MCP · via Kerf"}};
    const QJsonValue root;
    return QJsonObject{
        {"type", "comments"},
        {"reviewRevision", 41},
        {"items", QJsonArray{
             note("c-0300", root, mina, "2026-11-02T09:00:00Z", "Open on the road, not the sky", {{"at", 3.0}, {"frame", 72}}),
             note("c-0903", root, tom, "2026-11-02T10:00:00Z", "The sea reads magenta here", {{"at", 9.12}, {"frame", 219}, {"tags", QJsonArray{"color"}}}),
             note("c-0903-r1", "c-0903", kerf, "2026-11-02T10:01:00Z", "Sea back toward cyan"),
             note("c-1708", root, QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}, {"role", "QC"}}, "2026-11-03T08:00:00Z",
                  "Caption overlaps the lower-third by 8 px",
                  {{"at", 17.32}, {"duration", 2.48}, {"frame", 416}, {"tags", QJsonArray{"captions"}}, {"qcKey", "qc:text.overlap:clip-12"}}),
             note("c-1708-r1", "c-1708", tom, "2026-11-03T08:30:00Z", "Also clips the sun flare, circled",
                  {{"drawing", QJsonArray{QJsonObject{{"tool", "ellipse"},
                                                      {"points", QJsonArray{QJsonArray{0.61, 0.22}, QJsonArray{0.74, 0.35}}},
                                                      {"color", "#ededed"}}}}}),
             note("c-2614", root, mina, "2026-11-03T09:00:00Z", "Logo lands late, should hit on the snare",
                  {{"at", 16.56}, {"frame", 397}, {"blocking", true}}),
             note("c-2614-r1", "c-2614", claude, "2026-11-03T09:05:00Z", "Move logo reveal to the snare"),
             note("c-2718", root, qc, "2026-11-03T10:00:00Z", "True peak −0.4 dBTP, limit −1.0 dBTP (EBU R128)",
                  {{"at", 17.72}, {"duration", 0.28}, {"frame", 425}, {"blocking", true}, {"tags", QJsonArray{"audio"}},
                   {"qcKey", "qc:audio.true_peak"}}),
             note("c-2718-r1", "c-2718", you, "2026-11-03T10:05:00Z", "@Kerf limit the music bus here"),
         }}};
}

// The mockup's eight QC rows (mockup-spec 12).
QJsonObject seedCheck() {
    const auto row = [](const char *code, const char *name, bool passed, const char *value, const char *thread = nullptr) {
        QJsonObject out{{"code", code}, {"name", name}, {"passed", passed}, {"value", QString::fromUtf8(value)}};
        if (thread) out.insert("commentId", thread);
        return out;
    };
    return QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false}, {"findings", QJsonArray{}},
                       {"summary", QJsonObject{{"passed", 6}, {"total", 8}, {"blocking", 1}, {"rows", QJsonArray{
                           row("loudness.integrated", "Integrated loudness", true, "−23.1 LUFS"),
                           row("loudness.true_peak", "True peak", false, "−0.4 dBTP", "c-2718"),
                           row("loudness.range", "Loudness range", true, "6.2 LU"),
                           row("picture.out_of_range", "Gamut Rec.709", true, "legal"),
                           row("picture.luma_levels", "Luma levels", true, "Y′ 64–940"),
                           row("text.safe_area", "Title safe 90%", true, "0 outside"),
                           row("text.overlap", "Caption overlap", false, "1 at 00:00:17:08", "c-1708"),
                           row("picture.flash_black", "Flash and black", true, "0 · 0")}}}}};
}

QJsonObject seedLoudness() {
    QJsonArray series;
    for (int i = 0; i <= 180; ++i) {
        const double t = i / 10.0;
        const double s = -25.5 + 2.2 * std::sin(t / 3.1);
        series.append(QJsonArray{t, s + 1, t < 3 ? QJsonValue() : QJsonValue(s)});
    }
    return QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                       {"truePeak", QJsonArray{-0.4, -1.3}}, {"shortTermMax", -14.9}, {"momentaryMax", -12.0},
                       {"series", series}, {"target", -23}, {"limit", -1}, {"duration", 18.0}};
}

QJsonObject seedQueue() {
    return QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{
        QJsonObject{{"jobId", "j1"}, {"name", "atlas_draft"}, {"quality", "review"}, {"codec", "h264"}, {"status", "completed"},
                    {"progress", 1.0}, {"outputPath", "deliveries/atlas_draft.mp4"}},
        QJsonObject{{"jobId", "j2"}, {"name", "atlas_master"}, {"quality", "master"}, {"codec", "prores_hq"}, {"status", "running"},
                    {"progress", 0.42}}}}};
}

// The showcase cut in `canvas`'s shape, with the seed's proposals.
QJsonObject seedState(const QSize &canvas) {
    QJsonObject snapshot = showcaseSnapshot(22, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("width"), canvas.width());
    timeline.insert(QStringLiteral("height"), canvas.height());
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.insert(QStringLiteral("reviewRevision"), 41);
    return QJsonObject{{"type", "state"}, {"snapshot", snapshot}};
}

struct Format {
    const char *query; // the mockup's ?fmt=
    QSize canvas;
    double pictureHeight; // in the phone at the 1600 x 1000 stage (spec 3.2)
};

struct Expected {
    double bodyLeft;
    int railLeft;
    QRect panel;
    QRectF scrub;
    QRect source; // empty: no Source column
    double sourceWidthTolerance;
};

// The [OWNER] table (mockup-spec 3.1) at the 1296 x 564 stage, with the W3
// phone scrub: sx = rx - 372.
// PENDING OWNER CONFIRMATION: the phone Source column's 344 (open) and 836
// (closed) follow from the moved scrub through the "scrub - 32" rule; the
// spec's table says 400 at x 16 beside the open panel. Only the scrub was
// moved by the owner; these two widths wait for the owner's answer.
Expected expectedStage(bool phone, bool open) {
    if (!phone && open) return {85.0, 0, QRect(916, 8, 368, 548), QRectF(96.3, 500, 671.8, 56), QRect(), 0};
    if (!phone) return {533.5, 1240, QRect(), QRectF(544.8, 500, 671.8, 56), QRect(16, 8, 506, 548), 0.5};
    if (open) return {506.6, 748, QRect(804, 8, 480, 548), QRectF(376, 500, 360, 56), QRect(16, 8, 344, 548), 0};
    return {998.6, 1240, QRect(), QRectF(868, 500, 360, 56), QRect(16, 8, 836, 548), 0};
}

bool nearly(double actual, double expected, double tolerance) { return std::abs(actual - expected) <= tolerance; }

} // namespace

class MockupParityTest final : public QObject {
    Q_OBJECT

private slots:
    void init() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer"));
    }
    void cleanup() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer"));
    }

    void editorMatchesTheMockupInEveryState() {
        // Every kit motion lands at once, so each state is drawn settled.
        ui::setReducedMotionOverride(1);
        const auto motion = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (!shots.isEmpty()) QVERIFY(QDir().mkpath(shots));
        const Format formats[] = {{"169", QSize(1920, 1080), 0}, {"916", QSize(1080, 1920), 379.8},
                                  {"11", QSize(1080, 1080), 213.7}, {"45", QSize(1080, 1350), 267.1}};
        const char *workspaces[] = {"review", "edit", "finish", "deliver"};
        int states = 0;
        for (const Format &format : formats) {
            init();
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
            window.handleEvent(seedState(format.canvas));
            window.handleEvent(seedComments());
            window.handleEvent(seedCheck());
            window.handleEvent(seedLoudness());
            window.handleEvent(seedQueue());
            fakeArtwork(window.m_artwork);
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            window.m_errorBanner->hide();
            const QSize frameSize = format.canvas / 4;
            window.m_viewer->setFrame(testing::footageFrame(testing::Shot::HarborWide, frameSize), 0.0);
            const bool phone = format.canvas.width() <= format.canvas.height();
            QVERIFY(window.m_bus->review());
            QVERIFY(window.m_bus->review()->counts().open > 0);

            for (const char *ws : workspaces) {
                const QString name = QString::fromLatin1(ws);
                QAction *choose = window.m_commands->action(QStringLiteral("workspace.%1").arg(name));
                QVERIFY2(choose, ws);
                choose->trigger();
                QCoreApplication::processEvents();
                const Workspace::StageState stage = window.m_workspace->stageState();
                // mockup-spec 10: Edit closes the panel, Finish dims.
                QCOMPARE(stage.panelOpen, name != QLatin1String("edit"));
                QCOMPARE(stage.dim, name == QLatin1String("finish"));
                QCOMPARE(window.m_viewer->chatOpen(), stage.panelOpen);
                // Switching rings no button: the keys go to the timeline or the stage.
                for (ui::KitButton *button : window.findChildren<ui::KitButton *>())
                    QVERIFY2(!(button->isVisible() && button->focusVisible()),
                             qPrintable(QStringLiteral("%1: %2 shows its focus ring (focus %3, in window %4)")
                                            .arg(name, button->objectName(),
                                                 QApplication::focusWidget() ? QApplication::focusWidget()->objectName() : QStringLiteral("none"))
                                            .arg(window.isAncestorOf(button))));
                for (const bool toggled : {false, true}) {
                    const bool open = toggled ? !stage.panelOpen : stage.panelOpen;
                    if (window.m_viewer->chatOpen() != open) window.setChatOpen(open);
                    for (int i = 0; i < 3; ++i) QCoreApplication::processEvents();
                    const QString label = QStringLiteral("%1-%2%3").arg(name, QString::fromLatin1(format.query),
                                                                       toggled ? QStringLiteral("-panel%1").arg(open ? 1 : 0) : QString());
                    checkRegions(window, name, label);
                    checkStage(window, format, phone, open, name, label);
                    if (!shots.isEmpty()) {
                        const QImage image = window.grab().toImage();
                        QVERIFY2(image.save(QDir(shots).filePath(label + QStringLiteral(".png"))), qPrintable(label));
                    }
                    ++states;
                }
                // Back to the workspace's own stage before the next one.
                if (window.m_viewer->chatOpen() != stage.panelOpen) window.setChatOpen(stage.panelOpen);
            }
            window.close();
        }
        QCOMPARE(states, 4 * 4 * 2);
    }

    // A fresh launch opens in the saved workspace with its stage applied, as
    // choosing it does: Edit with the panel closed and the Source column
    // showing, Finish dimmed with its tray, Deliver with its tray.
    // The keys start on the timeline the first time the window is active;
    // coming back to the window later keeps the button a keyboard user
    // tabbed to (settleFocus runs on the first activation, not on each).
    void comingBackKeepsTheKeyboardsPlace() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        QVERIFY(window.m_focusSettled);
        QVERIFY(!qobject_cast<QAbstractButton *>(QApplication::focusWidget()));
        QAbstractButton *button = nullptr;
        for (QAbstractButton *candidate : window.findChildren<QAbstractButton *>())
            if (candidate->isVisible() && candidate->isEnabled() && (candidate->focusPolicy() & Qt::TabFocus)) {
                button = candidate;
                break;
            }
        QVERIFY(button);
        button->setFocus(Qt::TabFocusReason);
        QCOMPARE(QApplication::focusWidget(), button);
        // Another app came forward and the window is active again.
        QEvent activation(QEvent::ActivationChange);
        QCoreApplication::sendEvent(&window, &activation);
        QVERIFY(window.isActiveWindow());
        QCOMPARE(QApplication::focusWidget(), button);
        window.close();
    }

    void launchAppliesTheWorkspaceStage() {
        for (const char *ws : {"Edit", "Finish", "Deliver", "Review"}) {
            init();
            QSettings().setValue(QStringLiteral("workspace/current"), QString::fromLatin1(ws));
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(seedState(QSize(1920, 1080)));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            const QString name = QString::fromLatin1(ws);
            QCOMPARE(window.m_workspace->current(), name);
            QCOMPARE(window.m_bus->workspace(), name);
            QCOMPARE(window.m_viewer->chatOpen(), name != QLatin1String("Edit"));
            QCOMPARE(window.m_chat->isVisible(), name != QLatin1String("Edit"));
            QCOMPARE(window.m_stageSource->isVisible(), name == QLatin1String("Edit"));
            QCOMPARE(window.m_viewer->dimmed(), name == QLatin1String("Finish"));
            QCOMPARE(window.m_commands->action(QStringLiteral("workspace.%1").arg(name.toLower()))->isChecked(), true);
            QCOMPARE(window.m_commands->action(QStringLiteral("view.dim"))->isChecked(), name == QLatin1String("Finish"));
            const QImage image = window.grab().toImage();
            const QPoint origin = window.m_viewer->mapTo(&window, QPoint(4, 4));
            QCOMPARE(image.pixelColor(origin * image.devicePixelRatio()).rgb(),
                     tokens::color(name == QLatin1String("Finish") ? tokens::surroundDim : tokens::surround).rgb());
            window.close();
        }
        cleanup();
    }

private:
    // mockup-spec 0.1 at 1600 x 1000, each region with its 1 pt seam.
    void checkRegions(MainWindow &window, const QString &ws, const QString &label) {
        Workspace *workspace = window.m_workspace;
        const auto frame = [&window](const QWidget *widget) { return QRect(widget->mapTo(&window, QPoint()), widget->size()); };
        const auto area = [&](const char *id) { return frame(workspace->panel(QString::fromLatin1(id))->dockAreaWidget()); };
        const auto shown = [&](const char *id) { return !workspace->panel(QString::fromLatin1(id))->isClosed(); };
        const auto near = [](int actual, int expected) { return std::abs(actual - expected) <= 1; };
        const QRect toolbar = frame(window.m_toolbar);
        const QRect status = frame(window.statusBar());
        const QRect program = area("program");
        const QRect inspector = area("inspector");
        const QString got = QStringLiteral("%1: toolbar %2 status %3,%4 program %5,%6 %7x%8 inspector %9,%10 %11x%12")
                                .arg(label).arg(toolbar.height()).arg(status.top()).arg(status.height())
                                .arg(program.x()).arg(program.y()).arg(program.width()).arg(program.height())
                                .arg(inspector.x()).arg(inspector.y()).arg(inspector.width()).arg(inspector.height());
        QVERIFY2(toolbar == QRect(0, 0, 1600, 40), qPrintable(got));
        QVERIFY2(status == QRect(0, 976, 1600, 24), qPrintable(got));
        // Program x 0-1296, y 40-632: its 28 strip over the 564 stage.
        QVERIFY2(program.topLeft() == QPoint(0, 40) && program.width() == 1296 && near(program.height(), 592), qPrintable(got));
        QVERIFY2(frame(window.m_programStrip) == QRect(0, 40, 1296, 28), qPrintable(got));
        QVERIFY2(frame(window.m_viewer) == QRect(0, 68, 1296, 564), qPrintable(got));
        // The Inspector column x 1296-1600, as tall as Program.
        QVERIFY2(near(inspector.left(), 1297) && inspector.right() == 1599 && inspector.top() == 40 &&
                     inspector.height() == program.height(),
                 qPrintable(got));
        const bool tray = ws == QLatin1String("finish") || ws == QLatin1String("deliver");
        if (tray) {
            // Finish and Deliver: the tray in the whole band, x 0-1600, y 632-976.
            const char *own = ws == QLatin1String("finish") ? "finish" : "deliver";
            QVERIFY2(shown(own), qPrintable(label));
            for (const char *id : {"project", "timeline", "browser", ws == QLatin1String("finish") ? "deliver" : "finish"})
                QVERIFY2(!shown(id), qPrintable(QStringLiteral("%1: %2 shown").arg(label, QString::fromLatin1(id))));
            const QRect band = area(own);
            QVERIFY2(band.left() == 0 && band.right() == 1599 && near(band.top(), 633) && band.bottom() == 975,
                     qPrintable(QStringLiteral("%1: tray %2,%3 %4x%5").arg(label).arg(band.x()).arg(band.y()).arg(band.width()).arg(band.height())));
        } else {
            // The band y 632-976: bin x 0-272, timeline 272-1296, browser 1296-1600.
            for (const char *id : {"project", "timeline", "browser"}) QVERIFY2(shown(id), qPrintable(QStringLiteral("%1: %2").arg(label, QString::fromLatin1(id))));
            for (const char *id : {"finish", "deliver"}) QVERIFY2(!shown(id), qPrintable(QStringLiteral("%1: %2").arg(label, QString::fromLatin1(id))));
            const QRect bin = area("project");
            const QRect timeline = area("timeline");
            const QRect browser = area("browser");
            const QString band = QStringLiteral("%1: bin %2,%3 %4x%5 timeline %6 %7 browser %8 %9")
                                     .arg(label).arg(bin.x()).arg(bin.y()).arg(bin.width()).arg(bin.height())
                                     .arg(timeline.x()).arg(timeline.width()).arg(browser.x()).arg(browser.width());
            QVERIFY2(bin.left() == 0 && near(bin.top(), 633) && bin.bottom() == 975 && near(bin.width() + 1, 272), qPrintable(band));
            QVERIFY2(near(timeline.left(), 272) && near(timeline.width(), 1024) && timeline.top() == bin.top(), qPrintable(band));
            QVERIFY2(near(browser.left(), 1297) && browser.right() == 1599 && browser.top() == bin.top(), qPrintable(band));
        }
        // Deliver's Inspector shows the Deliver pane while Video is chosen.
        QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Inspector);
    }

    // The 3.1 [OWNER] placement and the 3.2 exact devices, in stage points.
    void checkStage(MainWindow &window, const Format &format, bool phone, bool open, const QString &ws, const QString &label) {
        ViewerWidget *viewer = window.m_viewer;
        QVERIFY2(!viewer->moving(), qPrintable(label));
        const ViewerWidget::StageLayout layout = viewer->stageLayout();
        const Expected want = expectedStage(phone, open);
        const QString got = QStringLiteral("%1: body %2,%3 %4x%5 rail %6 panel %7,%8 %9x%10 scrub %11,%12 %13 source %14,%15 %16x%17")
                                .arg(label).arg(layout.body.left()).arg(layout.body.top()).arg(layout.body.width()).arg(layout.body.height())
                                .arg(layout.rail.left()).arg(layout.panel.x()).arg(layout.panel.y()).arg(layout.panel.width())
                                .arg(layout.panel.height()).arg(layout.scrub.left()).arg(layout.scrub.top()).arg(layout.scrub.width())
                                .arg(layout.source.x()).arg(layout.source.y()).arg(layout.source.width()).arg(layout.source.height());
        QVERIFY2(!layout.overlay, qPrintable(got));
        QCOMPARE(layout.kind, phone ? ViewerWidget::Device::Phone : ViewerWidget::Device::Laptop);
        // The device, the rail 12 right of it, 44 x 548 from TOP 8.
        QVERIFY2(nearly(layout.body.left(), want.bodyLeft, 0.5) && layout.body.top() == 8.0, qPrintable(got));
        QVERIFY2(layout.rail.width() == 44 && layout.rail.height() == 548 && layout.rail.top() == 8, qPrintable(got));
        QVERIFY2(nearly(layout.railRect.left() - layout.body.right(), 12, 0.5), qPrintable(got));
        if (want.railLeft) QVERIFY2(layout.rail.left() == want.railLeft, qPrintable(got));
        else QVERIFY2(nearly(layout.railRect.left(), 791.5, 0.5), qPrintable(got));
        // The panel 12 from the stage's right edge, or none.
        QVERIFY2(layout.panel == want.panel, qPrintable(got));
        QCOMPARE(window.m_chat->isVisible(), open);
        if (open) QCOMPARE(window.m_chat->geometry(), layout.panel);
        // The scrub strip, 56 tall at SY 500: the picture's width under the
        // laptop; 360 under a phone, ending 12 left of the rail.
        QVERIFY2(nearly(layout.scrub.left(), want.scrub.left(), 0.5) && nearly(layout.scrub.width(), want.scrub.width(), 0.05) &&
                     layout.scrub.top() == 500.0 && layout.scrub.height() == 56.0,
                 qPrintable(got));
        QCOMPARE(layout.narrowScrub, phone);
        if (phone) QVERIFY2(nearly(layout.scrub.right(), layout.rail.left() - 12, 1e-6), qPrintable(got));
        QCOMPARE(window.m_stageWiring->scrub()->geometry(), layout.scrub.toRect());
        QCOMPARE(window.m_rail->geometry(), layout.rail);
        // The Source column on the left, or none.
        if (want.source.isEmpty()) {
            QVERIFY2(layout.source.isEmpty(), qPrintable(got));
            QVERIFY2(!window.m_stageSource->isVisible(), qPrintable(got));
        } else {
            QVERIFY2(layout.source.topLeft() == QPoint(16, 8) && layout.source.height() == 548 &&
                         nearly(layout.source.width(), want.source.width(), want.sourceWidthTolerance + 0.5),
                     qPrintable(got));
            QVERIFY2(window.m_stageSource->isVisible(), qPrintable(got));
            QCOMPARE(window.m_stageSource->geometry(), layout.source);
        }
        // The exact devices (spec 3.2), to within half a point.
        const double aspect = double(format.canvas.width()) / format.canvas.height();
        if (phone) {
            const DeviceGeometry device = deviceGeometry(layout.body, aspect);
            QVERIFY2(nearly(layout.body.width(), 229.4, 0.5) && nearly(layout.body.height(), 480, 0.5), qPrintable(got));
            QVERIFY2(nearly(device.screen.width(), 213.7, 0.5) && nearly(device.screen.height(), 464.5, 0.5), qPrintable(got));
            QVERIFY2(nearly(device.bezel, 7.87, 0.5) && nearly(device.topBezel, 7.73, 0.5), qPrintable(got));
            QVERIFY2(nearly(device.screenRadius, 33.0, 0.5) && nearly(device.bodyRadius, 40.8, 0.5), qPrintable(got));
            QVERIFY2(nearly(layout.card.width(), 213.7, 0.5) && nearly(layout.card.height(), format.pictureHeight, 0.5), qPrintable(got));
            QVERIFY2(nearly(layout.card.center().y(), device.screen.center().y(), 0.5), qPrintable(got));
            QCOMPARE(layout.screen, device.screen);
        } else {
            const LaptopGeometry laptop = laptopGeometry(layout.body, aspect);
            QVERIFY2(nearly(layout.body.width(), 694.5, 0.5) && nearly(layout.body.height(), 483, 0.5), qPrintable(got));
            QVERIFY2(nearly(laptop.lid.width(), 694.5, 0.5) && nearly(laptop.lid.height(), 459.0, 0.5), qPrintable(got));
            QVERIFY2(nearly(laptop.screen.width(), 671.8, 0.5) && nearly(laptop.screen.height(), 436.3, 0.5), qPrintable(got));
            QVERIFY2(nearly(laptop.base.width(), 694.5, 0.5) && nearly(laptop.base.height(), 24.0, 0.5), qPrintable(got));
            QVERIFY2(nearly(laptop.bezel, 11.33, 0.5) && nearly(laptop.topBezel, laptop.bezel, 1e-9), qPrintable(got));
            QVERIFY2(nearly(layout.card.width(), 671.8, 0.5) && nearly(layout.card.height(), 377.9, 0.5), qPrintable(got));
            // 29.2 of the screen's black above and below the 16:9 picture.
            QVERIFY2(nearly(layout.card.top() - laptop.screen.top(), 29.2, 0.5), qPrintable(got));
            QCOMPARE(layout.screen, laptop.screen);
        }
        // Dim: Finish's surround is surroundDim, the others' surround.
        const QImage image = window.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const QPoint origin = viewer->mapTo(&window, QPoint());
        const QPointF probe(layout.body.left() - 6, layout.body.top() + 8);
        QCOMPARE(image.pixelColor((QPointF(origin) + probe).toPoint() * ratio).rgb(),
                 tokens::color(ws == QLatin1String("finish") ? tokens::surroundDim : tokens::surround).rgb());
        // The rail's badge counts the seed's open threads.
        QVERIFY2(!window.m_rail->commentsBadge()->isHidden(), qPrintable(label));
        QCOMPARE(window.m_rail->commentsBadge()->text(), QString::number(window.m_bus->review()->counts().open));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::MockupParityTest)

#include "mockup_parity_test.moc"
