// The sidebar's folders and conversations: each folder unfolds to its own
// conversations, latest first, each with a dot for what it is doing and a word
// or a time at its end; the first few show, and archived ones fold away at the
// end; a search over their titles; archive, rename and delete, asked for only
// as the sidebar says; and a folder's own menu.

#include "test_registry.h"

#include "design_tokens.h"
#include "ui/components.h"
#include "ui/project_sidebar.h"

#include <QAction>
#include <QApplication>
#include <QDir>
#include <QSettings>
#include <QFile>
#include <QClipboard>
#include <QEnterEvent>
#include <QLineEdit>
#include <QMenu>
#include <QMessageBox>
#include <QPushButton>
#include <QScopeGuard>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <QTimer>

namespace kerf {

namespace {

using State = ProjectSidebar::State;

// 2026-11-16, noon, where the tests run.
QDateTime noon() { return QDateTime(QDate(2026, 11, 16), QTime(12, 0)); }

ProjectSidebar::Conversation conversation(const QString &id, const QString &title, State state, const QDateTime &updated,
                                          bool archived = false) {
    ProjectSidebar::Conversation c;
    c.id = id;
    c.title = title;
    c.state = state;
    c.updated = updated;
    c.archived = archived;
    c.count = 4;
    return c;
}

// The conversation rows in the order they are listed.
QStringList listed(const ProjectSidebar &sidebar) {
    QStringList titles;
    for (const SidebarRow *row : sidebar.rows())
        if (row->objectName() == QStringLiteral("sidebarConversation")) titles << row->text();
    return titles;
}

// Answers the delete question that is about to be asked: `confirm` presses
// Delete, else Cancel. What it asked is kept in `asked`.
void answerDelete(QObject *context, bool confirm, QString *asked) {
    QTimer::singleShot(0, context, [confirm, asked] {
        auto *box = qobject_cast<QMessageBox *>(QApplication::activeModalWidget());
        if (!box) QFAIL("no question");
        *asked = box->text() + QLatin1Char(' ') + box->informativeText();
        QAbstractButton *press = nullptr;
        for (QAbstractButton *button : box->buttons()) {
            const QMessageBox::ButtonRole role = box->buttonRole(button);
            if (confirm ? role == QMessageBox::DestructiveRole : role == QMessageBox::RejectRole) press = button;
        }
        if (!press) {
            box->close();
            QFAIL("no such button");
        }
        press->click();
    });
}

} // namespace

class SidebarTest final : public QObject {
    Q_OBJECT

    QTemporaryDir m_folder;

    // A sidebar on one folder, its clock held at noon, shown so rows lay out.
    void open(ProjectSidebar &sidebar, const QList<ProjectSidebar::Conversation> &conversations, const QString &current = {}) {
        sidebar.m_fixedNow = noon();
        sidebar.resize(tokens::layout::sidebar, 800);
        sidebar.setProjects({m_folder.path()}, m_folder.path());
        sidebar.setConversations(conversations, current);
        sidebar.show();
        QVERIFY(QTest::qWaitForWindowExposed(&sidebar));
    }

private slots:
    void initTestCase() { QVERIFY(m_folder.isValid()); }

    void theSidebarIsItsTokensWidth() {
        QCOMPARE(tokens::layout::sidebar, 280);
        // A drag of its edge keeps it between its least and what it leaves the
        // window, as wide as a wide window allows.
        QVERIFY(tokens::layout::sidebarLeast < tokens::layout::sidebar);
        QCOMPARE(tokens::layout::sidebarMost(1600), 1600 - tokens::layout::sidebarLeaves);
        QCOMPARE(tokens::layout::sidebarMost(600), tokens::layout::sidebarLeast);
        QVERIFY(tokens::layout::sidebar < tokens::layout::sidebarMost(tokens::layout::sidebarWindowLeast));
        SidebarRow row(QString(), QStringLiteral("A"));
        QCOMPARE(row.sizeHint().width(), tokens::layout::sidebar);
    }

    void timesSayHowLongAgo() {
        const QDateTime now = noon();
        QCOMPARE(ProjectSidebar::relativeTime(now, now), QStringLiteral("now"));
        QCOMPARE(ProjectSidebar::relativeTime(now.addSecs(-59), now), QStringLiteral("now"));
        // A clock a little ahead is still now.
        QCOMPARE(ProjectSidebar::relativeTime(now.addSecs(30), now), QStringLiteral("now"));
        QCOMPARE(ProjectSidebar::relativeTime(now.addSecs(-4 * 60), now), QStringLiteral("4m"));
        QCOMPARE(ProjectSidebar::relativeTime(now.addSecs(-2 * 3600 - 50), now), QStringLiteral("2h"));
        QCOMPARE(ProjectSidebar::relativeTime(now.addDays(-3), now), QStringLiteral("3d"));
        QCOMPARE(ProjectSidebar::relativeTime(now.addDays(-6), now), QStringLiteral("6d"));
        QCOMPARE(ProjectSidebar::relativeTime(QDateTime(QDate(2026, 10, 1), QTime(9, 0)), now), QStringLiteral("Oct 1"));
        QCOMPARE(ProjectSidebar::relativeTime(QDateTime(QDate(2025, 3, 4), QTime(9, 0)), now), QStringLiteral("Mar 2025"));
        QVERIFY(ProjectSidebar::relativeTime(QDateTime(), now).isEmpty());
    }

    void conversationsAreListedLatestFirst() {
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        open(sidebar, {conversation(QStringLiteral("old"), QStringLiteral("Old cut"), State::Idle, QDateTime(QDate(2026, 10, 1), QTime(9, 0))),
                       conversation(QStringLiteral("t1"), QStringLiteral("Earlier today"), State::Idle, now.addSecs(-3 * 3600)),
                       conversation(QStringLiteral("t2"), QStringLiteral("Just now"), State::Idle, now.addSecs(-4 * 60)),
                       conversation(QStringLiteral("y"), QStringLiteral("Last night"), State::Idle, QDateTime(QDate(2026, 11, 15), QTime(20, 0))),
                       conversation(QStringLiteral("w"), QStringLiteral("This week"), State::Idle, now.addDays(-3))});
        // Under their folder, with nothing between them: how long ago each
        // changed is in its tip, so the title has the row.
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("Just now"), QStringLiteral("Earlier today"), QStringLiteral("Last night"),
                                               QStringLiteral("This week"), QStringLiteral("Old cut")}));
        QVERIFY(sidebar.notes().isEmpty());
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("t2"))), QStringLiteral("Just now · 4m"));
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("t1"))), QStringLiteral("Earlier today · 3h"));
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("y"))), QStringLiteral("Last night · 16h"));
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("w"))), QStringLiteral("This week · 3d"));
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("old"))), QStringLiteral("Old cut · Oct 1"));
        // Each is under its folder's heading, as a coding agent lists threads:
        // the folder's name small and quiet with its chevron after it, and the
        // titles flush with it, with no dot before them and nothing after them
        // at rest.
        SidebarRow *folder = sidebar.projectRow(m_folder.path());
        // The open folder's name is a step brighter than the others'.
        QVERIFY(folder->isHeading() && folder->isStrong() && !folder->hasGuide());
        QVERIFY(folder->chevronRect().left() > folder->textLeft());
        for (const SidebarRow *row : sidebar.rows())
            if (row->objectName() == QStringLiteral("sidebarConversation")) {
                QVERIFY(!row->hasGuide() && !row->isHeading());
                QCOMPARE(row->textLeft(), folder->textLeft());
                QVERIFY(!row->badge().isValid());
                QCOMPARE(row->textRect().right(), row->width() - 8 - 1);
            }
        // The rows stand a row apart, with no gap for the guide to break on.
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t1"))->y() - sidebar.conversationRow(QStringLiteral("t2"))->y(),
                 sidebar.conversationRow(QStringLiteral("t2"))->height());

        // Time goes on: the times are read again.
        sidebar.m_fixedNow = now.addSecs(60 * 60);
        sidebar.refreshTimes();
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("t2"))), QStringLiteral("Just now · 1h"));

        // Conversations with no time are listed first, with none in their tip.
        sidebar.setConversations({conversation(QStringLiteral("b"), QStringLiteral("Timed"), State::Idle, now),
                                  conversation(QStringLiteral("a"), QStringLiteral("Untimed"), State::Idle, QDateTime())},
                                 {});
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("Untimed"), QStringLiteral("Timed")}));
        QCOMPARE(ui::tipName(sidebar.conversationRow(QStringLiteral("a"))), QStringLiteral("Untimed"));

        // A folder with none says so, in the rows' place.
        sidebar.setConversations({}, {});
        QCOMPARE(sidebar.notes(), QStringList{QStringLiteral("No conversations yet")});
    }

    // The first few of a folder's conversations show; the rest are a press away.
    void aLongFolderShowsItsLatestFirst() {
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        QList<ProjectSidebar::Conversation> many;
        for (int index = 0; index < 9; ++index)
            many << conversation(QStringLiteral("c%1").arg(index), QStringLiteral("Cut %1").arg(index), State::Idle, now.addSecs(-3600 * index));
        open(sidebar, many);
        QCOMPARE(listed(sidebar).size(), ProjectSidebar::kShownAtFirst);
        QCOMPARE(listed(sidebar).constFirst(), QStringLiteral("Cut 0"));
        SidebarRow *more = sidebar.moreRow(m_folder.path());
        QVERIFY(more);
        QCOMPARE(more->text(), QStringLiteral("Show 3 more"));
        more->click();
        QCOMPARE(listed(sidebar).size(), 9);
        QCOMPARE(sidebar.moreRow(m_folder.path())->text(), QStringLiteral("Show less"));
        sidebar.moreRow(m_folder.path())->click();
        QCOMPARE(listed(sidebar).size(), ProjectSidebar::kShownAtFirst);

        // The one that is open is never out of sight, nor what a search finds.
        sidebar.setConversations(many, QStringLiteral("c8"));
        QCOMPARE(listed(sidebar).size(), 9);
        QVERIFY(sidebar.conversationRow(QStringLiteral("c8"))->isCurrent());
        // With none chosen, no row is filled: New is an icon and a word, and
        // the folder's name is not a choice among conversations.
        sidebar.setConversations(many, QString());
        QCOMPARE(listed(sidebar).size(), ProjectSidebar::kShownAtFirst);
        QVERIFY(!sidebar.newConversationRow()->isCurrent());
        QCOMPARE(sidebar.newConversationRow()->text(), QStringLiteral("New"));
        QCOMPARE(sidebar.newConversationRow()->accessibleName(), QStringLiteral("New conversation"));
        QVERIFY(!sidebar.projectRow(m_folder.path())->isCurrent());
        sidebar.searchField()->setText(QStringLiteral("Cut"));
        QCOMPARE(listed(sidebar).size(), 9);
        QVERIFY(!sidebar.moreRow(m_folder.path()));
        // Six or fewer need no row for the rest.
        sidebar.searchField()->clear();
        sidebar.setConversations(many.first(ProjectSidebar::kShownAtFirst), QString());
        QVERIFY(!sidebar.moreRow(m_folder.path()));
    }

    // Every folder unfolds to its own conversations: the open one's from the
    // bridge, another's from what it keeps beside its project.
    void eachFolderUnfoldsToItsOwnConversations() {
        QTemporaryDir other;
        QVERIFY(other.isValid());
        QDir kept(other.path());
        QVERIFY(kept.mkpath(QStringLiteral(".kerf/conversations")));
        const auto keep = [&kept](const char *id, const char *title, const char *updated, bool archived, const char *file) {
            QFile out(kept.filePath(QStringLiteral(".kerf/conversations/%1").arg(QString::fromLatin1(file))));
            if (!out.open(QIODevice::WriteOnly)) return false;
            out.write(QStringLiteral("{\"id\":\"%1\",\"title\":\"%2\",\"updated\":\"%3\",\"archived\":%4,\"items\":[{},{}]}")
                          .arg(QString::fromLatin1(id), QString::fromLatin1(title), QString::fromLatin1(updated),
                               archived ? QStringLiteral("true") : QStringLiteral("false"))
                          .toUtf8());
            return true;
        };
        QVERIFY(keep("c-1", "Ferry ride", "2026-11-14T08:00:00.000000Z", false, "c-1.json"));
        QVERIFY(keep("c-2", "Market scene", "2026-11-15T08:00:00Z", false, "c-2.json"));
        QVERIFY(keep("c-3", "Put away", "2026-11-16T08:00:00Z", true, "c-3.json"));
        // Neither a conversation nor readable: left out, as the bridge leaves them.
        QVERIFY(keep("note", "Not one", "2026-11-16T08:00:00Z", false, "note.json"));
        QFile broken(kept.filePath(QStringLiteral(".kerf/conversations/c-9.json")));
        QVERIFY(broken.open(QIODevice::WriteOnly));
        broken.write("{ not json");
        broken.close();

        const QList<ProjectSidebar::Conversation> read = ProjectSidebar::readConversations(other.path());
        QCOMPARE(read.size(), 3);
        QCOMPARE(read.at(0).title, QStringLiteral("Put away"));
        QCOMPARE(read.at(1).title, QStringLiteral("Market scene"));
        QCOMPARE(read.at(1).count, 2);
        QVERIFY(read.at(2).updated.isValid());

        QSettings().remove(QStringLiteral("sidebar/folded"));
        ProjectSidebar sidebar;
        sidebar.m_fixedNow = noon();
        sidebar.resize(tokens::layout::sidebar, 800);
        sidebar.setProjects({m_folder.path(), other.path()}, m_folder.path());
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Here"), State::Idle, noon())}, QString());
        sidebar.show();
        QVERIFY(QTest::qWaitForWindowExposed(&sidebar));
        // The open folder shows what is in it; the other is folded until asked.
        QVERIFY(!sidebar.isFolded(m_folder.path()));
        QVERIFY(sidebar.isFolded(other.path()));
        QCOMPARE(listed(sidebar), QStringList{QStringLiteral("Here")});
        SidebarRow *folder = sidebar.projectRow(other.path());
        QCOMPARE(folder->chevron(), SidebarRow::Chevron::Folded);
        QCOMPARE(folder->accessibleDescription(), QStringLiteral("Folded"));

        // Its name, like its chevron, unfolds it: nothing is opened by that.
        QSignalSpy projectChosen(&sidebar, &ProjectSidebar::projectChosen);
        QSignalSpy chosenIn(&sidebar, &ProjectSidebar::conversationChosenIn);
        folder->click();
        QVERIFY(projectChosen.isEmpty());
        QVERIFY(!sidebar.isFolded(other.path()));
        QCOMPARE(sidebar.projectRow(other.path())->chevron(), SidebarRow::Chevron::Unfolded);
        // Its own, latest first; its archived ones stay out of the way.
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("Here"), QStringLiteral("Market scene"), QStringLiteral("Ferry ride")}));
        QCOMPARE(sidebar.conversationsOf(other.path()).size(), 3);
        // What was chosen is kept for the next sidebar.
        {
            ProjectSidebar next;
            next.setProjects({m_folder.path(), other.path()}, m_folder.path());
            QVERIFY(!next.isFolded(other.path()));
        }

        // One of them opens its project, on it; it has no archive or delete here.
        SidebarRow *theirs = nullptr;
        for (SidebarRow *row : sidebar.rows())
            if (row->text() == QStringLiteral("Market scene")) theirs = row;
        QVERIFY(theirs);
        QVERIFY(theirs->hoverActions().isEmpty());
        // How long ago, wherever the clock is: it was written in UTC, and noon is local.
        QCOMPARE(ui::tipName(theirs), QStringLiteral("Market scene · %1")
                                          .arg(ProjectSidebar::relativeTime(QDateTime::fromString(QStringLiteral("2026-11-15T08:00:00Z"), Qt::ISODate), noon())));
        theirs->click();
        QCOMPARE(chosenIn.size(), 1);
        QCOMPARE(chosenIn.first().at(0).toString(), QDir::cleanPath(other.path()));
        QCOMPARE(chosenIn.first().at(1).toString(), QStringLiteral("c-2"));

        // A press on the chevron alone folds it again, and Left and Right do from the keys.
        folder = sidebar.projectRow(other.path());
        QTest::mouseClick(folder, Qt::LeftButton, Qt::NoModifier, folder->chevronRect().center());
        QVERIFY(sidebar.isFolded(other.path()));
        QCOMPARE(listed(sidebar), QStringList{QStringLiteral("Here")});
        QTest::keyClick(sidebar.projectRow(other.path()), Qt::Key_Right);
        QVERIFY(!sidebar.isFolded(other.path()));
        QTest::keyClick(sidebar.projectRow(other.path()), Qt::Key_Left);
        QVERIFY(sidebar.isFolded(other.path()));
        // The open folder folds too.
        sidebar.projectRow(m_folder.path())->click();
        QVERIFY(sidebar.isFolded(m_folder.path()));
        QVERIFY(listed(sidebar).isEmpty());
        sidebar.setFolded(m_folder.path(), false);
        QSettings().remove(QStringLiteral("sidebar/folded"));
    }

    // A folder's menu: a conversation in it, the folder in the file manager,
    // its path, and off the list.
    void aFolderHasItsOwnMenu() {
        QTemporaryDir other;
        QVERIFY(other.isValid());
        ProjectSidebar sidebar;
        sidebar.resize(tokens::layout::sidebar, 800);
        sidebar.setProjects({m_folder.path(), other.path()}, m_folder.path());
        sidebar.show();
        QVERIFY(QTest::qWaitForWindowExposed(&sidebar));
        QSignalSpy newConversation(&sidebar, &ProjectSidebar::newConversationRequested);
        QSignalSpy chosenIn(&sidebar, &ProjectSidebar::conversationChosenIn);
        QSignalSpy opened(&sidebar, &ProjectSidebar::projectChosen);
        QSignalSpy removed(&sidebar, &ProjectSidebar::projectRemoveRequested);
        QSignalSpy revealed(&sidebar, &ProjectSidebar::projectRevealRequested);
        const auto action = [](QMenu *menu, const char *name) { return menu->findChild<QAction *>(QString::fromLatin1(name)); };

        // The open folder: it is open already, and stays on the list while it is.
        QMenu *menu = sidebar.projectMenu(m_folder.path());
        QVERIFY(menu);
        QVERIFY(!action(menu, "sidebarProjectOpen")->isVisible());
        QVERIFY(!action(menu, "sidebarProjectRemove")->isEnabled());
        action(menu, "sidebarProjectNew")->trigger();
        QCOMPARE(newConversation.size(), 1);
        action(menu, "sidebarProjectCopyPath")->trigger();
        QCOMPARE(QApplication::clipboard()->text(), QDir::toNativeSeparators(QDir::cleanPath(m_folder.path())));
        action(menu, "sidebarProjectReveal")->trigger();
        QCOMPARE(revealed.first().first().toString(), QDir::cleanPath(m_folder.path()));

        // Another: it can be opened, with a new conversation or as it is, and taken off.
        menu = sidebar.projectMenu(other.path());
        QVERIFY(action(menu, "sidebarProjectOpen")->isVisible());
        QVERIFY(action(menu, "sidebarProjectRemove")->isEnabled());
        action(menu, "sidebarProjectNew")->trigger();
        QCOMPARE(chosenIn.size(), 1);
        QVERIFY(chosenIn.first().at(1).toString().isEmpty());
        action(menu, "sidebarProjectOpen")->trigger();
        QCOMPARE(opened.first().first().toString(), QDir::cleanPath(other.path()));
        action(menu, "sidebarProjectRemove")->trigger();
        QCOMPARE(removed.first().first().toString(), QDir::cleanPath(other.path()));
        QVERIFY(!sidebar.projectMenu(QStringLiteral("/not/listed")));

        // The same from the row: + and its menu show under the pointer.
        SidebarRow *row = sidebar.projectRow(other.path());
        QCOMPARE(row->hoverActions().size(), 2);
        QVERIFY(!row->actionsShown());
        QEnterEvent enter(QPointF(60, 10), QPointF(60, 10), QPointF(60, 10));
        QApplication::sendEvent(row, &enter);
        QVERIFY(row->actionsShown());
        row->hoverActions().constFirst()->click();
        QCOMPARE(chosenIn.size(), 2);
        sidebar.projectRow(m_folder.path())->hoverActions().constFirst()->click();
        QCOMPARE(newConversation.size(), 2);

        // The start window lists folders alone: a press chooses one, and nothing folds.
        sidebar.setConversationsShown(false);
        row = sidebar.projectRow(other.path());
        QCOMPARE(row->chevron(), SidebarRow::Chevron::None);
        QVERIFY(row->hoverActions().isEmpty());
        opened.clear();
        row->click();
        QCOMPARE(opened.size(), 1);
    }

    // Working and making show on the title, as a light over it; done, waiting
    // for you and failed as a dot at the row's end; nothing going on, nothing.
    void eachStateShowsOnItsTitleOrItsEnd() {
        ProjectSidebar sidebar;
        const QDateTime then = noon().addSecs(-10 * 60);
        open(sidebar, {conversation(QStringLiteral("working"), QStringLiteral("Working"), State::Working, then),
                       conversation(QStringLiteral("making"), QStringLiteral("Making"), State::Making, then),
                       conversation(QStringLiteral("review"), QStringLiteral("Review"), State::Review, then),
                       conversation(QStringLiteral("settled"), QStringLiteral("Settled"), State::Settled, then),
                       conversation(QStringLiteral("failed"), QStringLiteral("Failed"), State::Failed, then),
                       conversation(QStringLiteral("idle"), QStringLiteral("Idle"), State::Idle, then)});
        struct Expect {
            const char *id;
            bool shimmer;
            QColor badge;
            const char *said;
        };
        using namespace tokens;
        const Expect expected[] = {
            {"working", true, QColor(), "Working, 4 messages"},
            {"making", true, QColor(), "Making, 4 messages"},
            {"review", false, color(activity::review), "Review, 4 messages"},
            {"settled", false, color(activity::settled), "Done, 4 messages"},
            {"failed", false, color(activity::failed), "Failed, 4 messages"},
            {"idle", false, QColor(), "4 messages"},
        };
        for (const Expect &e : expected) {
            SidebarRow *row = sidebar.conversationRow(QString::fromLatin1(e.id));
            QVERIFY2(row, e.id);
            QCOMPARE(row->isShimmering(), e.shimmer);
            QCOMPARE(row->badge(), e.badge);
            QCOMPARE(row->accessibleDescription(), QString::fromLatin1(e.said));
            // A dot keeps the title clear of it.
            QCOMPARE(row->textRect().right() < row->width() - 8 - 7, e.badge.isValid());
        }
        // The colours are the theme's: blue for what is there to look at, red
        // for what stopped.
        QCOMPARE(color(activity::review), color(accentInk));
        QCOMPARE(color(activity::settled), color(accentInk));
        QCOMPARE(color(activity::failed), color(neg));
    }

    // What finishes while you look elsewhere keeps a dot until you look at it.
    void whatFinishesUnseenKeepsADot() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Cut"), State::Working, noon()),
                       conversation(QStringLiteral("b"), QStringLiteral("Other"), State::Idle, noon())},
             QStringLiteral("a"));
        // Another window is the active one: it finishes unseen.
        QWidget elsewhere;
        elsewhere.resize(200, 100);
        elsewhere.show();
        elsewhere.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&elsewhere));
        QVERIFY(!sidebar.watching());
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Cut"), State::Idle, noon()),
                                  conversation(QStringLiteral("b"), QStringLiteral("Other"), State::Idle, noon())},
                                 QStringLiteral("a"));
        SidebarRow *row = sidebar.conversationRow(QStringLiteral("a"));
        QVERIFY(!row->isShimmering());
        QCOMPARE(row->badge(), tokens::color(tokens::activity::settled));
        QCOMPARE(row->accessibleDescription(), QStringLiteral("Done, 4 messages"));
        QVERIFY(!sidebar.conversationRow(QStringLiteral("b"))->badge().isValid());
        QVERIFY(sidebar.unseen().contains(QStringLiteral("a")));
        // Looked at, it is seen.
        sidebar.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&sidebar));
        QTRY_VERIFY(!sidebar.conversationRow(QStringLiteral("a"))->badge().isValid());
        QVERIFY(sidebar.unseen().isEmpty());
        // What finishes while you look at it is seen as it finishes.
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Cut"), State::Working, noon()),
                                  conversation(QStringLiteral("b"), QStringLiteral("Other"), State::Idle, noon())},
                                 QStringLiteral("a"));
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Cut"), State::Idle, noon()),
                                  conversation(QStringLiteral("b"), QStringLiteral("Other"), State::Idle, noon())},
                                 QStringLiteral("a"));
        QVERIFY(!sidebar.conversationRow(QStringLiteral("a"))->badge().isValid());
    }

    // The light over a working title runs with motion on, and only the title is
    // drawn again.
    void theWorkingTitleShimmersOnlyWithMotion() {
        {
            ProjectSidebar sidebar;
            open(sidebar, {conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon())});
            // Tests run with reduced motion: the title holds still, in ink.
            QVERIFY(sidebar.conversationRow(QStringLiteral("w"))->isShimmering());
            QCOMPARE(sidebar.m_shimmer.state(), QAbstractAnimation::Stopped);
        }
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon()),
                       conversation(QStringLiteral("i"), QStringLiteral("Idle"), State::Idle, noon())});
        QCOMPARE(sidebar.m_shimmer.state(), QAbstractAnimation::Running);
        sidebar.m_shimmer.setCurrentTime(tokens::motion::titleShimmer / 2);
        SidebarRow *working = sidebar.conversationRow(QStringLiteral("w"));
        QVERIFY(working->m_phase > 0.4 && working->m_phase < 0.6);
        QCOMPARE(sidebar.conversationRow(QStringLiteral("i"))->m_phase, 0.0);
        // What is drawn again is the title, not the row.
        QVERIFY(working->textRect().left() > 0 && working->textRect().right() < working->width() - 1);
        // Nothing working, nothing drawn.
        sidebar.setConversations({conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Settled, noon())}, {});
        QCOMPARE(sidebar.m_shimmer.state(), QAbstractAnimation::Stopped);
        // Hidden, it stops too.
        sidebar.setConversations({conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon())}, {});
        QCOMPARE(sidebar.m_shimmer.state(), QAbstractAnimation::Running);
        sidebar.hide();
        QCOMPARE(sidebar.m_shimmer.state(), QAbstractAnimation::Stopped);
    }

    // A folded folder Kerf works in says so on its name.
    void aFoldedFolderShimmersWhileKerfWorks() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon())});
        sidebar.setWorking(true);
        QVERIFY(!sidebar.projectRow(m_folder.path())->isShimmering());
        sidebar.setFolded(m_folder.path(), true);
        QVERIFY(sidebar.projectRow(m_folder.path())->isShimmering());
        sidebar.setWorking(false);
        QVERIFY(!sidebar.projectRow(m_folder.path())->isShimmering());
        // The fold is kept: leave the folder open for the next sidebar.
        sidebar.setFolded(m_folder.path(), false);
    }

    void archivedConversationsFoldAwayAtTheEnd() {
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Kept"), State::Idle, now.addSecs(-60)),
                       conversation(QStringLiteral("b"), QStringLiteral("Put away"), State::Settled, now.addDays(-2), true),
                       conversation(QStringLiteral("c"), QStringLiteral("Long gone"), State::Idle, now.addDays(-30), true)});
        QCOMPARE(listed(sidebar), QStringList{QStringLiteral("Kept")});
        SidebarRow *fold = sidebar.archivedRow();
        QVERIFY(fold);
        QCOMPARE(fold->text(), QStringLiteral("Archived · 2"));
        QCOMPARE(sidebar.rows().last(), fold);
        QVERIFY(!sidebar.conversationRow(QStringLiteral("b")));

        fold->click();
        QVERIFY(sidebar.archivedOpen());
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("Kept"), QStringLiteral("Put away"), QStringLiteral("Long gone")}));
        SidebarRow *archived = sidebar.conversationRow(QStringLiteral("b"));
        QVERIFY(archived->isDimmed());
        QVERIFY(!sidebar.conversationRow(QStringLiteral("a"))->isDimmed());
        // Still openable.
        QSignalSpy chosen(&sidebar, &ProjectSidebar::conversationChosen);
        archived->click();
        QCOMPARE(chosen.value(0).value(0).toString(), QStringLiteral("b"));
        // And taken out again from its menu.
        QAction *unarchive = sidebar.conversationMenu(QStringLiteral("b"))->findChild<QAction *>(QStringLiteral("sidebarArchive"));
        QCOMPARE(unarchive->text(), QStringLiteral("Unarchive"));
        QSignalSpy archiving(&sidebar, &ProjectSidebar::archiveRequested);
        unarchive->trigger();
        QCOMPARE(archiving.size(), 1);
        QCOMPARE(archiving.value(0).value(0).toString(), QStringLiteral("b"));
        QCOMPARE(archiving.value(0).value(1).toBool(), false);
        QCOMPARE(sidebar.archivedRow()->text(), QStringLiteral("Archived · 1"));
        QVERIFY(!sidebar.conversationRow(QStringLiteral("b"))->isDimmed());

        // Folded again, and none left: no row for it at all.
        sidebar.archivedRow()->click();
        QVERIFY(!sidebar.archivedOpen());
        QVERIFY(!sidebar.conversationRow(QStringLiteral("c")));
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Kept"), State::Idle, now)}, {});
        QVERIFY(!sidebar.archivedRow());
    }

    // Under the pointer, the row's end has its menu, where its dot was.
    void hoverShowsTheMenu() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Review, noon().addSecs(-120))});
        SidebarRow *row = sidebar.conversationRow(QStringLiteral("a"));
        QCOMPARE(row->hoverActions().size(), 1);
        QCOMPARE(row->hoverActions().at(0)->accessibleName(), QStringLiteral("More for Make a cut"));
        QVERIFY(!row->actionsShown());
        const int rest = row->textRect().right();
        QEnterEvent enter(QPointF(20, 10), QPointF(20, 10), QPointF(20, 10));
        QApplication::sendEvent(row, &enter);
        QVERIFY(row->actionsShown());
        // At the row's end, inside it, and the title keeps clear of it.
        const QRect more = row->hoverActions().first()->geometry();
        QVERIFY(row->rect().contains(more));
        QVERIFY(more.right() > row->width() - 2 * tokens::space::s8);
        QVERIFY(row->textRect().right() < more.left());
        QVERIFY(row->textRect().right() <= rest);
        // It opens the conversation's menu, over it.
        row->hoverActions().first()->click();
        QTRY_VERIFY(sidebar.m_conversationMenu->isVisible());
        QCOMPARE(sidebar.m_menuFor, QStringLiteral("a"));
        sidebar.m_conversationMenu->close();
        QEvent leave(QEvent::Leave);
        QApplication::sendEvent(row, &leave);
        QVERIFY(!row->actionsShown());
    }

    void deleteAsksFirst() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon()),
                       conversation(QStringLiteral("b"), QStringLiteral("Another"), State::Idle, noon())});
        QSignalSpy deleting(&sidebar, &ProjectSidebar::deleteRequested);
        QString asked;
        answerDelete(&sidebar, false, &asked);
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarDelete"))->trigger();
        QCOMPARE(asked, QStringLiteral("Delete “Make a cut”? This can't be undone."));
        QCOMPARE(deleting.size(), 0);
        QVERIFY(sidebar.conversationRow(QStringLiteral("a")));

        answerDelete(&sidebar, true, &asked);
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarDelete"))->trigger();
        QCOMPARE(deleting.size(), 1);
        QCOMPARE(deleting.value(0).value(0).toString(), QStringLiteral("a"));
        QVERIFY(!sidebar.conversationRow(QStringLiteral("a")));
        QVERIFY(sidebar.conversationRow(QStringLiteral("b")));

        // From the keyboard too, with the same question.
        SidebarRow *row = sidebar.conversationRow(QStringLiteral("b"));
        answerDelete(&sidebar, true, &asked);
        QTest::keyClick(row, Qt::Key_Delete);
        QCOMPARE(asked, QStringLiteral("Delete “Another”? This can't be undone."));
        QCOMPARE(deleting.size(), 2);
    }

    void theMenuRenamesArchivesAndDeletes() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon()),
                       conversation(QStringLiteral("z"), QStringLiteral("Put away"), State::Idle, noon(), true)});
        QMenu *menu = sidebar.conversationMenu(QStringLiteral("a"));
        QVERIFY(menu);
        QStringList texts;
        for (QAction *action : menu->actions())
            if (!action->isSeparator()) texts << action->text();
        QCOMPARE(texts, (QStringList{QStringLiteral("Rename…"), QStringLiteral("Archive"), QStringLiteral("Delete…")}));
        QCOMPARE(ui::menuRole(menu->findChild<QAction *>(QStringLiteral("sidebarDelete"))), ui::MenuRole::Warn);
        QCOMPARE(sidebar.conversationMenu(QStringLiteral("z"))->findChild<QAction *>(QStringLiteral("sidebarArchive"))->text(),
                 QStringLiteral("Unarchive"));

        QSignalSpy archiving(&sidebar, &ProjectSidebar::archiveRequested);
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarArchive"))->trigger();
        QCOMPARE(archiving.size(), 1);
        QCOMPARE(archiving.value(0).value(1).toBool(), true);
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarArchive"))->trigger();
        QCOMPARE(archiving.size(), 2);
        QCOMPARE(archiving.value(1).value(1).toBool(), false);

        QSignalSpy deleting(&sidebar, &ProjectSidebar::deleteRequested);
        QString asked;
        answerDelete(&sidebar, true, &asked);
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarDelete"))->trigger();
        QCOMPARE(deleting.size(), 1);

        // The menu key, or Shift+F10, opens it over the row.
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon())}, {});
        SidebarRow *row = sidebar.conversationRow(QStringLiteral("a"));
        QTest::keyClick(row, Qt::Key_F10, Qt::ShiftModifier);
        QTRY_VERIFY(sidebar.m_conversationMenu->isVisible());
        sidebar.m_conversationMenu->close();
    }

    void renameEditsInPlace() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon())});
        QSignalSpy renaming(&sidebar, &ProjectSidebar::renameRequested);

        // Esc leaves it as it was.
        sidebar.conversationMenu(QStringLiteral("a"))->findChild<QAction *>(QStringLiteral("sidebarRename"))->trigger();
        QLineEdit *field = sidebar.m_renameField;
        QVERIFY(field);
        QCOMPARE(field->parentWidget(), sidebar.conversationRow(QStringLiteral("a")));
        QCOMPARE(field->text(), QStringLiteral("Make a cut"));
        QVERIFY(sidebar.conversationRow(QStringLiteral("a"))->hoverActions().first()->isHidden());
        QTest::keyClicks(field, QStringLiteral("Other"));
        QTest::keyClick(field, Qt::Key_Escape);
        QVERIFY(!sidebar.m_renameField);
        QCOMPARE(renaming.size(), 0);
        QCOMPARE(sidebar.conversationRow(QStringLiteral("a"))->text(), QStringLiteral("Make a cut"));

        // Enter keeps it; F2 starts it from the keyboard.
        QTest::keyClick(sidebar.conversationRow(QStringLiteral("a")), Qt::Key_F2);
        field = sidebar.m_renameField;
        QVERIFY(field);
        field->selectAll();
        QTest::keyClicks(field, QStringLiteral("  The beach cut "));
        QTest::keyClick(field, Qt::Key_Return);
        QVERIFY(!sidebar.m_renameField);
        QCOMPARE(renaming.size(), 1);
        QCOMPARE(renaming.value(0).value(0).toString(), QStringLiteral("a"));
        QCOMPARE(renaming.value(0).value(1).toString(), QStringLiteral("The beach cut"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("a"))->text(), QStringLiteral("The beach cut"));

        // An empty title, or the same one, asks for nothing.
        sidebar.beginRename(QStringLiteral("a"));
        sidebar.m_renameField->clear();
        QTest::keyClick(sidebar.m_renameField, Qt::Key_Return);
        sidebar.beginRename(QStringLiteral("a"));
        QTest::keyClick(sidebar.m_renameField, Qt::Key_Return);
        QCOMPARE(renaming.size(), 1);

        // A new list while typing keeps what was typed.
        sidebar.beginRename(QStringLiteral("a"));
        sidebar.m_renameField->setText(QStringLiteral("Half typed"));
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("The beach cut"), State::Working, noon())}, {});
        QVERIFY(sidebar.m_renameField);
        QCOMPARE(sidebar.m_renameField->text(), QStringLiteral("Half typed"));
        QCOMPARE(sidebar.m_renameField->parentWidget(), sidebar.conversationRow(QStringLiteral("a")));
        QTest::keyClick(sidebar.m_renameField, Qt::Key_Escape);
    }

    void searchFiltersTitles() {
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("The Beach cut"), State::Idle, now),
                       conversation(QStringLiteral("b"), QStringLiteral("Captions"), State::Idle, now.addDays(-2)),
                       conversation(QStringLiteral("c"), QStringLiteral("beach at dusk"), State::Idle, now.addDays(-20), true)});
        QLineEdit *search = sidebar.searchField();
        // Search is a row under New conversation; pressed, the field takes its place.
        QVERIFY(!search->isVisible() && sidebar.searchRow()->isVisible());
        sidebar.resize(tokens::layout::sidebar, 600);
        sidebar.show();
        QVERIFY(QTest::qWaitForWindowExposed(&sidebar));
        const QRect rowWas = sidebar.searchRow()->geometry();
        sidebar.searchRow()->click();
        QVERIFY(search->isVisible() && !sidebar.searchRow()->isVisible());
        QCOMPARE(search->accessibleName(), QStringLiteral("Search conversations"));
        // The field is the row, typed into: where it was and as tall, so
        // nothing under it moves, and with nothing round it, neither a fill
        // nor an outline: the sidebar shows through to its edges.
        QCoreApplication::processEvents();
        QCOMPARE(search->geometry(), rowWas);
        QCOMPARE(search->placeholderText(), QStringLiteral("Search"));
        // The editor rings a field being typed in; this one asks for none.
        QVERIFY(search->property("ringsOwnFields").toBool());
        {
            const QImage shown = sidebar.grab().toImage();
            const QColor ground = tokens::color(tokens::sidebar);
            const QRect at = search->geometry();
            for (const QPoint &point : {at.topLeft() + QPoint(1, 1), at.bottomRight() - QPoint(1, 1), QPoint(at.right() - 2, at.center().y()),
                                        QPoint(at.center().x(), at.top()), QPoint(at.center().x(), at.bottom())})
                QCOMPARE(shown.pixelColor(point).rgb(), ground.rgb());
        }
        QTest::keyClicks(search, QStringLiteral("BEACH"));
        QCOMPARE(listed(sidebar), QStringList{QStringLiteral("The Beach cut")});
        QCOMPARE(sidebar.archivedRow()->text(), QStringLiteral("Archived · 1"));
        // The folders stay.
        QVERIFY(sidebar.projectRow(m_folder.path()));

        search->setText(QStringLiteral("nothing like it"));
        QVERIFY(listed(sidebar).isEmpty());
        QVERIFY(!sidebar.archivedRow());
        QCOMPARE(sidebar.notes(), QStringList{QStringLiteral("No conversations match")});

        // Esc empties it, and everything is back; Esc again closes it.
        QTest::keyClick(search, Qt::Key_Escape);
        QVERIFY(search->text().isEmpty());
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("The Beach cut"), QStringLiteral("Captions")}));
        QTest::keyClick(search, Qt::Key_Escape);
        QVERIFY(!search->isVisible() && sidebar.searchRow()->isVisible());
        QCOMPARE(sidebar.archivedRow()->text(), QStringLiteral("Archived · 1"));

        // The start window lists folders alone: no search there.
        sidebar.setConversationsShown(false);
        QVERIFY(!search->isVisible());
    }

    // KERF_SCREENSHOT_DIR=<dir> saves the sidebar with one of everything, to look at.
    void aPictureOfItWhenAsked() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("KERF_SCREENSHOT_DIR is not set");
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a 60-second cut of the harbour"), State::Working, now),
                       conversation(QStringLiteral("b"), QStringLiteral("Colour the beach take"), State::Review, now.addSecs(-600)),
                       conversation(QStringLiteral("c"), QStringLiteral("Render for Instagram"), State::Making, now.addSecs(-1200)),
                       conversation(QStringLiteral("d"), QStringLiteral("Captions for the vlog"), State::Settled, now.addDays(-1)),
                       conversation(QStringLiteral("e"), QStringLiteral("Loudness pass"), State::Failed, now.addDays(-3)),
                       conversation(QStringLiteral("f"), QStringLiteral("First rough cut"), State::Idle, now.addDays(-40)),
                       conversation(QStringLiteral("g"), QStringLiteral("Old title ideas"), State::Idle, now.addDays(-50), true)},
                 QStringLiteral("b"));
        sidebar.setAccount(QStringLiteral("You"), QStringLiteral("Kerf Cloud"));
        sidebar.setArchivedOpen(true);
        QEnterEvent enter(QPointF(20, 10), QPointF(20, 10), QPointF(20, 10));
        QApplication::sendEvent(sidebar.conversationRow(QStringLiteral("d")), &enter);
        QTest::qWait(50);
        QVERIFY(sidebar.grab().save(QDir(shots).filePath(QStringLiteral("sidebar.png"))));
        sidebar.beginRename(QStringLiteral("f"));
        QVERIFY(sidebar.grab().save(QDir(shots).filePath(QStringLiteral("sidebar-rename.png"))));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::SidebarTest)
#include "sidebar_test.moc"
