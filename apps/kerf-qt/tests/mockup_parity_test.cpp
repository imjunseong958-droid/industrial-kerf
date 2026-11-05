// The whole editor against the approved minimal mockup (03-review-thread.html)
// at its 1600 x 1000 canvas: in 16:9 and 9:16, the base
// state, each of the three sheets, and both trays, fed the seed (comments,
// proposals, a check, loudness and a render queue) as kerfd sends it, through
// handleEvent. Each state's rects are checked against the brief's table, with no
// gaps, cards, radii or margins around the three regions.
//
// With KERF_SCREENSHOT_DIR set, every state is saved under the mockup's query
// state: <fmt>.png for ?fmt=, <fmt>-media.png, <fmt>-browser.png and
// <fmt>-inspector.png for the sheets, <fmt>-finish.png and <fmt>-deliver.png for
// ?ws=, and <fmt>-preview.png for ?preview=1.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "components.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "main_window.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "ui/editor_bus.h"
#include "ui/inspector_column.h"
#include "ui/review_model.h"
#include "ui/wiring/review_wiring.h"
#include "ui/agent_hud.h"
#include "ui/comment_row.h"
#include "ui/deliver_tray.h"
#include "ui/browser_panel.h"
#include "ui/finish_tray.h"
#include "ui/proposal_card.h"
#include "ui/wiring/stage_wiring.h"
#include "ui/wiring/timeline_wiring.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"


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
    QRect slot; // the viewer
    QRect picture;
    QRect conversation;
    QRect timeline;
};

// The layout at 1600 x 1000, in window coordinates (ui/stage_shell.h).
const Format kFormats[] = {
    {"169", QSize(1920, 1080), QRect(640, 40, 960, 420), QRect(768, 52, 704, 396), QRect(240, 40, 400, 960), QRect(640, 460, 960, 540)},
    {"916", QSize(1080, 1920), QRect(640, 40, 960, 614), QRect(954, 52, 332, 590), QRect(240, 40, 400, 960), QRect(640, 654, 960, 346)},
};

QRect inWindow(QWidget *widget, QWidget *window) { return QRect(widget->mapTo(window, QPoint()), widget->size()); }

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
        int states = 0;
        for (const Format &format : kFormats) {
            MainWindow window{QString(), QString()};
            // An agent that can take requests, as the mockup's composer shows.
            window.m_cloudAccess = QStringLiteral("ready");
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
            window.m_viewer->setFrame(testing::footageFrame(testing::Shot::HarborWide, format.canvas / 2), 0.0);
            QVERIFY(window.m_bus->review());
            QVERIFY(window.m_bus->review()->counts().open > 0);
            const QString fmt = QString::fromLatin1(format.query);
            const auto save = [&window, &shots, &states](const QString &label) {
                for (int i = 0; i < 3; ++i) QCoreApplication::processEvents();
                ++states;
                if (shots.isEmpty()) return true;
                return window.grab().toImage().save(QDir(shots).filePath(label + QStringLiteral(".png")));
            };

            // The base state: the picture, the conversation and the timeline.
            QCoreApplication::processEvents();
            checkBase(window, format);
            checkNothingElseShows(window);
            // One painted hairline between the regions.
            {
                const QImage shot = window.grab().toImage();
                const int chrome = tokens::color(tokens::chrome).red();
                QVERIFY(shot.pixelColor(format.conversation.left(), format.conversation.top() + 100).red() > chrome);
                // Over the timeline under a wide picture, at its left beside a vertical one.
                const QPoint timelineSeam = format.timeline.top() > 40 ? format.timeline.topLeft() + QPoint(100, 0) : format.timeline.topLeft() + QPoint(0, 100);
                QVERIFY(shot.pixelColor(timelineSeam).red() > chrome);
            }
            QVERIFY(save(fmt));

            // The sheets, each at the window's right edge and the conversation's height.
            const struct {
                StageShell::Sheet sheet;
                const char *name;
                const char *command;
                QWidget *widget;
                int width;
            } sheets[] = {{StageShell::Sheet::Media, "media", "window.project", window.m_projectPanel, 272},
                          {StageShell::Sheet::Browser, "browser", "window.effects", window.m_browserPanel, 304},
                          {StageShell::Sheet::Inspector, "inspector", "window.inspector", window.m_inspectorColumn, 304}};
            for (const auto &sheet : sheets) {
                window.triggerCommand(QString::fromLatin1(sheet.command));
                QCOMPARE(window.m_shell->sheet(), sheet.sheet);
                QVERIFY2(sheet.widget->isVisible(), sheet.name);
                QCOMPARE(inWindow(sheet.widget, &window),
                         QRect(format.conversation.right() + 1 - sheet.width, format.conversation.top(), sheet.width, format.conversation.height()));
                // What is under it has not moved.
                checkBase(window, format);
                QVERIFY(save(QStringLiteral("%1-%2").arg(fmt, QString::fromLatin1(sheet.name))));
                window.triggerCommand(QString::fromLatin1(sheet.command));
                QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
            }

            // The trays, 344 tall across the full window at the bottom.
            const struct {
                StageShell::Tray tray;
                const char *name;
                QWidget *widget;
            } trays[] = {{StageShell::Tray::Finish, "finish", window.m_finishTray}, {StageShell::Tray::Deliver, "deliver", window.m_deliverTray}};
            for (const auto &tray : trays) {
                window.triggerCommand(QStringLiteral("workspace.%1").arg(QString::fromLatin1(tray.name)));
                QCOMPARE(window.m_shell->tray(), tray.tray);
                QVERIFY2(tray.widget->isVisible(), tray.name);
                QCOMPARE(inWindow(tray.widget, &window), QRect(240, 656, 1360, 344));
                checkBase(window, format);
                // Changing state rings no button: the keys go to the timeline.
                for (ui::KitButton *button : window.findChildren<ui::KitButton *>())
                    QVERIFY2(!(button->isVisible() && button->focusVisible()), qPrintable(button->objectName()));
                QVERIFY(save(QStringLiteral("%1-%2").arg(fmt, QString::fromLatin1(tray.name))));
                window.triggerCommand(QStringLiteral("workspace.review"));
                QCOMPARE(window.m_shell->tray(), StageShell::Tray::None);
            }

            // ?preview=1: the chip, 12 inside the picture's top left.
            window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}, {"proposalId", "p7"}});
            PreviewChip *chip = window.m_stageWiring->previewChip();
            QVERIFY(chip->isVisible());
            QCOMPARE(chip->text(), QStringLiteral("Previewing proposal v1"));
            // .ptxt: the 6 pt ink2 dot 16 in, the text 8 past it in ink3.
            auto *dot = chip->findChild<ui::Dot *>(QStringLiteral("previewChipDot"));
            QVERIFY(dot);
            QCOMPARE(dot->color(), tokens::color(tokens::ink2));
            QCOMPARE(dot->geometry(), QRect(16, 9, 6, 6));
            auto *chipText = chip->findChild<QLabel *>(QStringLiteral("previewChipText"));
            QCOMPARE(chipText->geometry().left(), 30);
            QVERIFY(chipText->styleSheet().contains(tokens::color(tokens::ink3).name()));
            QCOMPARE(chip->height(), 24);
            QCOMPARE(inWindow(chip, &window).topLeft(), format.picture.topLeft() + QPoint(12, 12));
            QVERIFY(save(QStringLiteral("%1-preview").arg(fmt)));
            window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "exited"}});
            QVERIFY(!chip->isVisible());

            // Rows and proposal cards fill the panel's full width, at every format.
            checkConversationRows(window);
            window.close();
        }
        QCOMPARE(states, 2 * (1 + 3 + 2 + 1));
    }

    // The timeline at the design's size: its strip, heads, default heights, and a
    // scale that fits the sequence to the lanes.
    void timelineMatchesTheMockup() {
        for (const Format &format : kFormats) {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(seedState(format.canvas));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            TimelineToolStrip *strip = window.m_timelineWiring->tools();
            QCOMPARE(strip->height(), 28);
            QCOMPARE(inWindow(strip, &window), QRect(format.timeline.topLeft(), QSize(format.timeline.width(), 28)));
            QCOMPARE(window.m_timeline->trackHeaderRect(0).width(), 140);
            QVERIFY(window.m_timeline->isFitToWidth());
            QCOMPARE(window.m_timeline->wideSequence(), format.canvas.width() > format.canvas.height());
            QCOMPARE(window.m_timelineScroll->viewport()->height(), format.timeline.height() - 28);
            window.close();
        }
    }

    // The real launch: shown and activated while connecting (the timeline is
    // disabled, so Qt hands the keys to the next control), then state and
    // ready in the bridge's order. The timeline's keys must work.
    void theKeysStartOnTheTimeline() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        QVERIFY(!window.m_timeline->isEnabled());
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_timeline->hasFocus());
        // Space plays, C takes the razor, I sets the mark.
        window.m_lastCommand = {};
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_Space);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("play"));
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_C);
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Razor);
        QVERIFY(!window.m_stageWiring->hasIn());
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_I);
        QVERIFY(window.m_stageWiring->hasIn());
        // A reconnection takes the same path.
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "gone"}});
        QVERIFY(!window.m_timeline->hasFocus());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_timeline->hasFocus());
        window.close();
    }

    // Coming ready leaves the keys with someone typing, and with a sheet in use.
    void comingReadyLeavesTypingAndSheetsAlone() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        window.triggerCommand(QStringLiteral("window.project"));
        window.m_projectPanel->focusSearch();
        QLineEdit *search = window.m_projectPanel->m_search;
        QVERIFY(search->hasFocus());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(search->hasFocus());
        // Not a text field, but in the open sheet.
        QAbstractButton *inSheet = nullptr;
        for (QAbstractButton *button : window.m_projectPanel->findChildren<QAbstractButton *>())
            if (button->isVisible() && button->isEnabled()) inSheet = button;
        QVERIFY(inSheet);
        inSheet->setFocus();
        window.settleFocus();
        QCOMPARE(window.focusWidget(), inSheet);
        // The sheet closed, they go back to the timeline.
        window.triggerCommand(QStringLiteral("window.project"));
        QVERIFY(window.m_timeline->hasFocus());
        window.close();
    }

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
        QVERIFY(window.m_timeline->hasFocus());
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

private:
    void checkBase(MainWindow &window, const Format &format) {
        QCOMPARE(window.m_toolbar->height(), 40);
        QCOMPARE(inWindow(window.m_viewer, &window), format.slot);
        // With no margin: the frame fills its region's height, or its width too.
        QCOMPARE(window.m_viewer->videoRect(), QRectF(format.picture.translated(-format.slot.topLeft())));
        QCOMPARE(inWindow(window.m_chat, &window), format.conversation);
        QCOMPARE(inWindow(window.m_timelinePanel, &window), format.timeline);
        QVERIFY(window.m_viewer->isVisible());
        QVERIFY(window.m_chat->isVisible());
        QVERIFY(window.m_timelinePanel->isVisible());
    }

    void checkNothingElseShows(MainWindow &window) {
        for (QWidget *closed : {static_cast<QWidget *>(window.m_projectPanel), static_cast<QWidget *>(window.m_browserPanel),
                                static_cast<QWidget *>(window.m_inspectorColumn), static_cast<QWidget *>(window.m_finishTray),
                                static_cast<QWidget *>(window.m_deliverTray)})
            QVERIFY2(!closed->isVisible(), qPrintable(closed->objectName()));
        QVERIFY(!window.findChild<QStatusBar *>());
        QVERIFY(window.m_stageWiring->previewChip()->isHidden());
        QVERIFY(window.m_stageWiring->hud()->isHidden());
        // Safe zones are off by default.
        QVERIFY(!window.m_viewer->safeZonesShown());
    }

    void checkConversationRows(MainWindow &window) {
        ChatPanel *chat = window.m_chat;
        chat->m_content->layout()->activate();
        QCoreApplication::processEvents();
        const int width = chat->scroll()->viewport()->width();
        QCOMPARE(chat->m_content->width(), width);
        // Every thread and card stands as wide as the list, whatever its width.
        const QList<CommentRow *> rows = chat->m_content->findChildren<CommentRow *>();
        QVERIFY(!rows.isEmpty());
        // (A row keeps the design's 8 to the list's edge at every width.)
        // Within the column it keeps to read in, when it has more room than that.
        const int inset = std::max(0, (chat->width() - tokens::layout::conversationColumn) / 2);
        for (const CommentRow *row : rows)
            if (row->isVisibleTo(chat)) QCOMPARE(row->mapTo(chat->m_content, QPoint(row->width(), 0)).x(), width - 8 - inset);
        const QList<ProposalCard *> cards = chat->m_content->findChildren<ProposalCard *>();
        for (const ProposalCard *card : cards) {
            if (!card->isVisibleTo(chat)) continue;
            // Its gutters are its row's own, not a max-width's.
            const int right = card->mapTo(chat->m_content, QPoint(card->width(), 0)).x();
            QVERIFY2(right >= width - 16 - inset, qPrintable(QStringLiteral("%1 of %2").arg(right).arg(width)));
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::MockupParityTest)

#include "mockup_parity_test.moc"
