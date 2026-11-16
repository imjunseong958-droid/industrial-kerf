// The sidebar's conversations: grouped by date, each with a dot for what it is
// doing and a word or a time at its end; archived ones folded away at the end;
// a search over their titles; and archive, rename and delete, asked for only
// as the sidebar says.

#include "test_registry.h"

#include "design_tokens.h"
#include "ui/components.h"
#include "ui/project_sidebar.h"

#include <QAction>
#include <QApplication>
#include <QDir>
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
        QCOMPARE(tokens::layout::sidebar, 252);
        SidebarRow row(QString(), QStringLiteral("A"));
        QCOMPARE(row.sizeHint().width(), 252);
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

    void conversationsAreGroupedByDateLatestFirst() {
        ProjectSidebar sidebar;
        const QDateTime now = noon();
        open(sidebar, {conversation(QStringLiteral("old"), QStringLiteral("Old cut"), State::Idle, QDateTime(QDate(2026, 10, 1), QTime(9, 0))),
                       conversation(QStringLiteral("t1"), QStringLiteral("Earlier today"), State::Idle, now.addSecs(-3 * 3600)),
                       conversation(QStringLiteral("t2"), QStringLiteral("Just now"), State::Idle, now.addSecs(-4 * 60)),
                       conversation(QStringLiteral("y"), QStringLiteral("Last night"), State::Idle, QDateTime(QDate(2026, 11, 15), QTime(20, 0))),
                       conversation(QStringLiteral("w"), QStringLiteral("This week"), State::Idle, now.addDays(-3))});
        QCOMPARE(sidebar.sectionLabels(),
                 (QStringList{QStringLiteral("Today"), QStringLiteral("Yesterday"), QStringLiteral("Previous 7 days"), QStringLiteral("Older")}));
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("Just now"), QStringLiteral("Earlier today"), QStringLiteral("Last night"),
                                               QStringLiteral("This week"), QStringLiteral("Old cut")}));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t2"))->trailing(), QStringLiteral("4m"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t1"))->trailing(), QStringLiteral("3h"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("y"))->trailing(), QStringLiteral("16h"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("w"))->trailing(), QStringLiteral("3d"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("old"))->trailing(), QStringLiteral("Oct 1"));

        // Only the groups that have rows are labelled.
        sidebar.setConversations({conversation(QStringLiteral("t2"), QStringLiteral("Just now"), State::Idle, now.addSecs(-4 * 60)),
                                  conversation(QStringLiteral("old"), QStringLiteral("Old cut"), State::Idle, now.addDays(-40))},
                                 {});
        QCOMPARE(sidebar.sectionLabels(), (QStringList{QStringLiteral("Today"), QStringLiteral("Older")}));

        // Time goes on: the times are read again, and a new day regroups.
        sidebar.m_fixedNow = now.addSecs(60 * 60);
        sidebar.refreshTimes();
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t2"))->trailing(), QStringLiteral("1h"));
        sidebar.m_fixedNow = now.addDays(1);
        sidebar.refreshTimes();
        QCOMPARE(sidebar.sectionLabels(), (QStringList{QStringLiteral("Yesterday"), QStringLiteral("Older")}));

        // Conversations with no time are listed first, under no label.
        sidebar.setConversations({conversation(QStringLiteral("a"), QStringLiteral("Untimed"), State::Idle, QDateTime())}, {});
        QVERIFY(sidebar.sectionLabels().isEmpty());
        QVERIFY(sidebar.conversationRow(QStringLiteral("a"))->trailing().isEmpty());
    }

    void eachStateHasItsDotAndWord() {
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
            QRgb dot;
            const char *trailing;
            QRgb trailingColor;
        };
        using namespace tokens;
        const Expect expected[] = {
            {"working", activity::working, "Working", accentInk}, {"making", activity::making, "Making", activity::making},
            {"review", activity::review, "Review", accentInk},    {"settled", activity::settled, "10m", ink3},
            {"failed", activity::failed, "10m", ink3},            {"idle", activity::idle, "10m", ink3},
        };
        for (const Expect &e : expected) {
            SidebarRow *row = sidebar.conversationRow(QString::fromLatin1(e.id));
            QVERIFY2(row, e.id);
            QCOMPARE(row->dotColor(), color(e.dot));
            QCOMPARE(row->trailing(), QString::fromLatin1(e.trailing));
            QCOMPARE(row->m_trailingColor, color(e.trailingColor));
            QCOMPARE(row->isPulsing(), QByteArray(e.id) == "working");
        }
        // The colours are the theme's, and each is its own.
        QCOMPARE(color(activity::working), color(accent));
        QCOMPARE(color(activity::settled), color(pos));
        QCOMPARE(color(activity::failed), color(neg));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("review"))->accessibleDescription(), QStringLiteral("Review, 4 messages"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("failed"))->accessibleDescription(), QStringLiteral("Failed, 4 messages"));
    }

    // The working dot pulses with motion on, and only it is drawn again.
    void theWorkingDotPulsesOnlyWithMotion() {
        {
            ProjectSidebar sidebar;
            open(sidebar, {conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon())});
            // Tests run with reduced motion: the dot holds still.
            QCOMPARE(sidebar.m_pulse.state(), QAbstractAnimation::Stopped);
            QCOMPARE(sidebar.conversationRow(QStringLiteral("w"))->m_pulse, 1.0);
        }
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon()),
                       conversation(QStringLiteral("i"), QStringLiteral("Idle"), State::Idle, noon())});
        QCOMPARE(sidebar.m_pulse.state(), QAbstractAnimation::Running);
        sidebar.m_pulse.setCurrentTime(tokens::motion::workPulse / 2);
        SidebarRow *working = sidebar.conversationRow(QStringLiteral("w"));
        QVERIFY(working->m_pulse < 1.0);
        QCOMPARE(sidebar.conversationRow(QStringLiteral("i"))->m_pulse, 1.0);
        QVERIFY(working->dotRect().width() <= 8);
        // Nothing working, nothing drawn.
        sidebar.setConversations({conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Settled, noon())}, {});
        QCOMPARE(sidebar.m_pulse.state(), QAbstractAnimation::Stopped);
        // Hidden, it stops too.
        sidebar.setConversations({conversation(QStringLiteral("w"), QStringLiteral("Working"), State::Working, noon())}, {});
        QCOMPARE(sidebar.m_pulse.state(), QAbstractAnimation::Running);
        sidebar.hide();
        QCOMPARE(sidebar.m_pulse.state(), QAbstractAnimation::Stopped);
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
        // And taken out again from its own button.
        ui::IconButton *unarchive = archived->hoverActions().first();
        QCOMPARE(unarchive->accessibleName(), QStringLiteral("Unarchive conversation"));
        QSignalSpy archiving(&sidebar, &ProjectSidebar::archiveRequested);
        unarchive->click();
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

    void hoverShowsArchiveAndDelete() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon().addSecs(-120))});
        SidebarRow *row = sidebar.conversationRow(QStringLiteral("a"));
        QCOMPARE(row->hoverActions().size(), 2);
        QCOMPARE(row->hoverActions().at(0)->accessibleName(), QStringLiteral("Archive conversation"));
        QCOMPARE(row->hoverActions().at(1)->accessibleName(), QStringLiteral("Delete conversation"));
        QVERIFY(!row->actionsShown());
        QEnterEvent enter(QPointF(20, 10), QPointF(20, 10), QPointF(20, 10));
        QApplication::sendEvent(row, &enter);
        QVERIFY(row->actionsShown());
        // At the row's end, inside it.
        const QRect last = row->hoverActions().last()->geometry();
        QVERIFY(row->rect().contains(last));
        QVERIFY(last.right() > row->width() - 2 * tokens::space::s8);
        QEvent leave(QEvent::Leave);
        QApplication::sendEvent(row, &leave);
        QVERIFY(!row->actionsShown());

        QSignalSpy archiving(&sidebar, &ProjectSidebar::archiveRequested);
        row->hoverActions().first()->click();
        QCOMPARE(archiving.size(), 1);
        QCOMPARE(archiving.value(0).value(0).toString(), QStringLiteral("a"));
        QCOMPARE(archiving.value(0).value(1).toBool(), true);
        QVERIFY(sidebar.conversations().first().archived);
        QCOMPARE(sidebar.archivedRow()->text(), QStringLiteral("Archived · 1"));
    }

    void deleteAsksFirst() {
        ProjectSidebar sidebar;
        open(sidebar, {conversation(QStringLiteral("a"), QStringLiteral("Make a cut"), State::Idle, noon()),
                       conversation(QStringLiteral("b"), QStringLiteral("Another"), State::Idle, noon())});
        QSignalSpy deleting(&sidebar, &ProjectSidebar::deleteRequested);
        QString asked;
        answerDelete(&sidebar, false, &asked);
        sidebar.conversationRow(QStringLiteral("a"))->hoverActions().last()->click();
        QCOMPARE(asked, QStringLiteral("Delete “Make a cut”? This can't be undone."));
        QCOMPARE(deleting.size(), 0);
        QVERIFY(sidebar.conversationRow(QStringLiteral("a")));

        answerDelete(&sidebar, true, &asked);
        sidebar.conversationRow(QStringLiteral("a"))->hoverActions().last()->click();
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
        QVERIFY(search->isVisible());
        QCOMPARE(search->accessibleName(), QStringLiteral("Search conversations"));
        QTest::keyClicks(search, QStringLiteral("BEACH"));
        QCOMPARE(listed(sidebar), QStringList{QStringLiteral("The Beach cut")});
        QCOMPARE(sidebar.sectionLabels(), QStringList{QStringLiteral("Today")});
        QCOMPARE(sidebar.archivedRow()->text(), QStringLiteral("Archived · 1"));
        // The folders stay.
        QVERIFY(sidebar.projectRow(m_folder.path()));

        search->setText(QStringLiteral("nothing like it"));
        QVERIFY(listed(sidebar).isEmpty());
        QVERIFY(!sidebar.archivedRow());
        QCOMPARE(sidebar.m_extras.size(), 1);

        // Esc empties it, and everything is back.
        QTest::keyClick(search, Qt::Key_Escape);
        QVERIFY(search->text().isEmpty());
        QCOMPARE(listed(sidebar), (QStringList{QStringLiteral("The Beach cut"), QStringLiteral("Captions")}));
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
