// MainWindow's comments panel (the agent chat), proposals, Ask and next
// steps, and ReviewWiring's members that reach into the window (Q-COMMENTS).
//
// The Kerf conversation lives in the review log: kerfd records each agent
// turn as the reader's comment and Kerf's reply (bridge.h). While a turn
// runs, the panel shows it as local rows (the request, then Kerf drafting
// with the stream's latest status); the comments event that records the
// turn takes their place.

#include "main_window.h"
#include "project_sidebar.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "comment_row.h"
#include "components.h"
#include "composer.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "proposal_card.h"
#include "review_link.h"
#include "review_model.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "stage_shell.h"
#include "wiring/main_window_shared.h"
#include "wiring/review_wiring.h"

#include <QApplication>
#include <QClipboard>
#include <QDateTime>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QJsonDocument>
#include <QLabel>
#include <QListWidget>
#include <QMenu>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QScrollBar>
#include <QSettings>
#include <QSignalBlocker>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QUrl>
#include <QVBoxLayout>


#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf {

using namespace window_parts;

namespace {

// A script line's words, a quoted phrase ("A lower third") one word.
QStringList scriptWords(const QString &line) {
    QStringList words;
    QString word;
    bool quoted = false;
    for (const QChar c : line) {
        if (c == QLatin1Char('"')) quoted = !quoted;
        if (c.isSpace() && !quoted) {
            if (!word.isEmpty()) words << std::exchange(word, QString());
            continue;
        }
        word += c;
    }
    if (!word.isEmpty()) words << word;
    return words;
}

// Seconds in a script's amount or position ("-36f", "+1.5s", "96f",
// "00:00:12:00"), or NaN when it is something else (an anchor).
double scriptSeconds(QString text, double fps) {
    text = text.trimmed();
    double sign = 1.0;
    if (text.startsWith(QLatin1Char('-')) || text.startsWith(QLatin1Char('+'))) {
        sign = text.startsWith(QLatin1Char('-')) ? -1.0 : 1.0;
        text = text.mid(1);
    }
    bool ok = false;
    double value = 0.0;
    if (text.endsWith(QLatin1Char('f'))) value = text.chopped(1).toDouble(&ok) / std::max(1.0, fps);
    else if (text.endsWith(QLatin1Char('s'))) value = text.chopped(1).toDouble(&ok);
    else {
        const QStringList parts = text.split(QRegularExpression(QStringLiteral("[:;]")));
        if (parts.size() == 4) {
            ok = true;
            double fields[4];
            for (int index = 0; index < 4; ++index) {
                bool part = false;
                fields[index] = parts.at(index).toDouble(&part);
                ok = ok && part;
            }
            value = fields[0] * 3600 + fields[1] * 60 + fields[2] + fields[3] / std::max(1.0, fps);
        }
    }
    return ok ? sign * value : std::numeric_limits<double>::quiet_NaN();
}

// "0:12", "1:04:30": a position as a reader says it.
QString clockTime(double seconds) {
    const qint64 whole = std::llround(std::max(0.0, seconds));
    const qint64 hours = whole / 3600;
    const QString rest = QStringLiteral("%1:%2").arg((whole / 60) % 60).arg(whole % 60, 2, 10, QLatin1Char('0'));
    return hours ? QStringLiteral("%1:%2").arg(hours).arg(rest.rightJustified(5, QLatin1Char('0'))) : rest;
}

// "−1.5 s", "+12 s": an amount, its sign always shown (a true minus).
QString signedSeconds(double seconds) {
    const QString magnitude = QString::number(std::abs(seconds), 'f', std::abs(seconds) < 10 ? 1 : 0);
    const QString trimmed = magnitude.endsWith(QStringLiteral(".0")) ? magnitude.chopped(2) : magnitude;
    return QStringLiteral("%1%2 s").arg(seconds < 0 ? QStringLiteral("−") : QStringLiteral("+"), trimmed);
}

// The value of `key=` among a line's words, unquoted.
QString scriptValue(const QStringList &words, const QString &key) {
    const QString prefix = key + QLatin1Char('=');
    for (const QString &word : words)
        if (word.startsWith(prefix)) {
            QString value = word.mid(prefix.size());
            if (value.size() >= 2 && value.startsWith(QLatin1Char('"')) && value.endsWith(QLatin1Char('"'))) value = value.mid(1, value.size() - 2);
            return value;
        }
    return {};
}

} // namespace

// ======================================================================= ReviewWiring

void ReviewWiring::attach(ChatPanel *panel) {
    m_panel = panel;
    MainWindow *w = m_window;
    m_bus->setReview(m_model);
    panel->setReview(m_model);
    // Who "you" are can arrive after the first of your comments does.
    connect(m_model, &ReviewModel::changed, panel, &ChatPanel::reconcileLocal);
    panel->setScriptDescriber([w](const QString &line) { return w->describeScriptLine(line); });
    panel->setReviewLink([this](double seconds) { return reviewLink(seconds); });

    // The bus feeds the model and the panel.
    connect(m_bus, &EditorBus::event, this, &ReviewWiring::handleEvent);
    connect(m_bus, &EditorBus::playheadChanged, this, [this](double seconds) {
        m_model->setPlayhead(seconds);
        if (m_panel) m_panel->setPlayhead(seconds);
    });
    connect(m_bus, &EditorBus::snapshotChanged, this, [this] {
        if (!m_panel) return;
        m_panel->setFps(m_bus->fps());
        m_panel->setNamedTimeline(m_bus->timeline());
    });
    connect(m_bus, &EditorBus::viewAsChanged, this, [this](const QString &person) { m_model->setViewAs(person); });
    // Cross-region requests.
    connect(m_bus, &EditorBus::openThread, this, [this](const QString &id) {
        if (m_panel) m_panel->openThread(id);
    });
    connect(m_bus, &EditorBus::askPrefill, this, [this](const QString &text, const QString &threadId) {
        if (!m_panel) return;
        if (!threadId.isEmpty()) {
            const ReviewModel::Thread *thread = m_model->threadOf(threadId);
            if (thread) {
                m_panel->openThread(thread->id, false);
                m_panel->setReplyTarget(thread->id);
            }
        }
        // Filled where it can be seen.
        m_window->uncoverComposer();
        m_panel->composerWidget()->fill(text);
    });
    connect(m_bus, &EditorBus::drawingFinished, this, [this](const QJsonArray &strokes) {
        if (m_panel) m_panel->composerWidget()->setDraw(strokes.isEmpty() ? Composer::Draw::None : Composer::Draw::Attached, strokes);
    });
    connect(m_bus, &EditorBus::drawingCancelled, this, [this] {
        if (m_panel && m_panel->composerWidget()->drawState() == Composer::Draw::Drawing)
            m_panel->composerWidget()->setDraw(Composer::Draw::None);
    });
    connect(m_model, &ReviewModel::currentChanged, m_bus, &EditorBus::currentThreadChanged);
    connect(panel, &ChatPanel::threadHovered, m_bus, &EditorBus::threadHovered);
    connect(panel, &ChatPanel::commentSelected, m_bus, &EditorBus::commentSelected);
    connect(m_model, &ReviewModel::changed, this, &ReviewWiring::syncPinnedProposal);
    connect(m_model, &ReviewModel::changed, m_bus, &EditorBus::reviewChanged);

    // What the panel asks for.
    connect(panel, &ChatPanel::postRequested, this, &ReviewWiring::post);
    connect(panel, &ChatPanel::seekRequested, w, &MainWindow::queueSeek);
    connect(panel, &ChatPanel::resolveRequested, this, &ReviewWiring::resolveThread);
    connect(panel, &ChatPanel::acceptRequested, this, &ReviewWiring::accept);
    connect(panel, &ChatPanel::rejectRequested, this, &ReviewWiring::reject);
    connect(panel, &ChatPanel::previewRequested, this, &ReviewWiring::preview);
    // ⌘↩ and Esc act on the version the pinned card shows.
    connect(panel, &ChatPanel::shownVersionChanged, this, &ReviewWiring::syncPinnedProposal);
    // The composer's Stop, Esc while busy, and the drafting row's Stop: one abort each.
    connect(panel, &ChatPanel::stopRequested, w, [w] {
        if (!w->execute(QStringLiteral("ai.stop"))) return;
        w->m_abortButton->setEnabled(false);
        w->appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Stopping request…"));
    });
    connect(panel, &ChatPanel::drawRequested, this, [this] {
        m_panel->composerWidget()->setDraw(Composer::Draw::Drawing);
        emit m_bus->drawingRequested();
    });
    connect(panel, &ChatPanel::drawCancelRequested, this, [this] {
        m_panel->composerWidget()->setDraw(Composer::Draw::None);
        emit m_bus->drawingCancelled();
    });
    connect(panel, &ChatPanel::runRequested, this, [this](const QString &what) {
        if (what == QLatin1String("qc")) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("check")}});
    });
    connect(panel, &ChatPanel::askAgainInThread, this, [this, w](const QString &text, const QString &threadId) {
        if (w->m_agentBusy || !w->m_connected || text.trimmed().isEmpty()) return;
        const ReviewModel::Thread *thread = threadId.isEmpty() ? nullptr : m_model->thread(threadId);
        const QString parent = thread && thread->origin == ReviewModel::Origin::Record ? thread->id : QString();
        QJsonObject comment;
        if (!parent.isEmpty()) comment.insert(QStringLiteral("parent"), parent);
        sendTurn(text, w->m_lastFocus, w->m_lastFocusLabel, comment, parent);
    });
    connect(panel, &ChatPanel::rowAction, this, [this](const QString &action, const QString &commentId) {
        if (action == QLatin1String("marker")) convertToMarker(commentId);
        else if (action == QLatin1String("delete")) deleteComment(commentId);
    });
    connect(panel, &ChatPanel::moreAction, this, [this, w](const QString &action, const QString &argument) {
        if (action == QLatin1String("settings")) {
            w->triggerCommand(QStringLiteral("app.preferences"));
        } else if (action == QLatin1String("activity")) {
            w->showActivity();
        } else if (action == QLatin1String("viewAs")) {
            m_model->setViewAs(argument);
            m_bus->setViewAs(argument);
        } else if (action == QLatin1String("model")) {
            w->m_agentModel = argument;
            m_panel->setModels(w->m_agentModels, argument, true);
        } else if (action == QLatin1String("export")) {
            QString path;
            if (argument != QLatin1String("markers")) {
                const QString suffix = argument == QLatin1String("csv") ? QStringLiteral("csv") : QStringLiteral("edl");
                const QString base = w->m_projectDirectory.isEmpty() ? QDir::homePath() : w->m_projectDirectory;
                path = QFileDialog::getSaveFileName(w, QStringLiteral("Export comments"),
                                                    QDir(base).filePath(QStringLiteral("comments.%1").arg(suffix)),
                                                    suffix == QLatin1String("csv") ? QStringLiteral("CSV (*.csv)")
                                                                                   : QStringLiteral("EDL (*.edl)"));
                if (path.isEmpty()) return;
            }
            exportComments(argument, path);
        }
    });

    panel->setFps(m_bus->fps());
    panel->setPlayhead(m_bus->playhead());
}

bool ReviewWiring::post() {
    MainWindow *w = m_window;
    if (!m_panel) return false;
    Composer *composer = m_panel->composerWidget();
    const QString text = composer->text().trimmed();
    if (text.isEmpty() || !w->m_connected) return false;
    // Saving an edit of one's own note.
    if (!m_panel->editing().isEmpty()) {
        if (!editComment(m_panel->editing(), text)) return false;
        composer->clear();
        m_panel->setEditing(QString());
        return true;
    }
    const QString reply = m_panel->replyTarget();
    // A reply goes to a review-log thread; an unlinked proposal's thread
    // (not in the log) only lends its range to Kerf's focus.
    const ReviewModel::Thread *thread = reply.isEmpty() ? nullptr : m_model->thread(reply);
    const QString parent = thread && thread->origin == ReviewModel::Origin::Record ? thread->id : QString();
    const bool stamped = composer->stamped() && parent.isEmpty();
    const double at = w->m_playhead;
    const QJsonArray drawing = composer->drawing();
    bool sent = false;
    if (review::addressesKerf(text)) {
        if (w->m_agentBusy) return false;
        // About the replied thread's mark, or the playhead.
        QJsonObject focus;
        QString label;
        if (thread && thread->root.hasAt()) {
            const double end = thread->root.at + std::max(0.0, thread->root.duration);
            focus = QJsonObject{{QStringLiteral("scope"), thread->root.duration > 0 ? QStringLiteral("range") : QStringLiteral("sequence")},
                                {QStringLiteral("fromFrame"), w->frameAt(thread->root.at)},
                                {QStringLiteral("toFrame"), w->frameAt(end)}};
            label = QStringLiteral("Sequence at %1").arg(review::markText(thread->root.at, thread->root.duration, w->m_fps));
        } else {
            focus = QJsonObject{{QStringLiteral("scope"), QStringLiteral("sequence")},
                                {QStringLiteral("fromFrame"), w->frameAt(at)},
                                {QStringLiteral("toFrame"), w->frameAt(at)}};
            label = QStringLiteral("Sequence at %1").arg(timelineTimecode(at, w->m_fps));
        }
        QJsonObject comment;
        if (stamped) comment.insert(QStringLiteral("at"), at);
        if (!parent.isEmpty()) comment.insert(QStringLiteral("parent"), parent);
        if (!drawing.isEmpty()) comment.insert(QStringLiteral("drawing"), drawing);
        sent = sendTurn(text, focus, label, comment, parent);
    } else {
        // A note for people (and agents over MCP): the review log only.
        const QString id = nextClientId();
        QJsonObject command{{QStringLiteral("type"), QStringLiteral("comment_add")},
                            {QStringLiteral("id"), id},
                            {QStringLiteral("body"), text}};
        if (stamped) command.insert(QStringLiteral("at"), at);
        if (!parent.isEmpty()) command.insert(QStringLiteral("parent"), parent);
        if (!drawing.isEmpty()) command.insert(QStringLiteral("drawing"), drawing);
        sent = m_bus->send(command);
        if (sent) m_panel->addPendingNote(id, text, parent, stamped ? at : std::numeric_limits<double>::quiet_NaN());
    }
    if (!sent) return false;
    composer->clear();
    m_panel->setReplyTarget(QString());
    w->updateChatControls();
    return true;
}

bool ReviewWiring::sendTurn(const QString &text, const QJsonObject &focus, const QString &focusLabel, const QJsonObject &comment,
                            const QString &threadId) {
    MainWindow *w = m_window;
    QJsonObject request = w->agentSettingsPayload();
    request.insert(QStringLiteral("type"), QStringLiteral("agent"));
    request.insert(QStringLiteral("text"), text);
    if (!focus.isEmpty()) request.insert(QStringLiteral("focus"), focus);
    // kerfd keeps the turn in the review log: the request as the reader's
    // comment, Kerf's answer as its reply.
    request.insert(QStringLiteral("comment"), comment);
    // The AI bar names what Kerf works on once the bridge reports it busy.
    w->m_agentFocusLabel = focusLabel;
    if (!m_bus->send(request)) return false;
    w->m_lastFocus = focus;
    w->m_lastFocusLabel = focusLabel;
    w->showNextSteps(QJsonArray{});
    w->m_agentBusy = true;
    w->m_agentText.clear();
    w->m_abortButton->setEnabled(true);
    if (m_panel) {
        const bool stamped = threadId.isEmpty() && comment.contains(QStringLiteral("at"));
        m_panel->beginTurn(text, threadId,
                           stamped ? comment.value(QStringLiteral("at")).toDouble() : std::numeric_limits<double>::quiet_NaN());
        m_panel->setTurnBusy(true);
        m_panel->setTurnStatus(focusLabel);
        w->m_agentRow = m_panel->turnKerfRow(false);
    }
    w->m_pendingReplies.clear();
    w->addActivity(QStringLiteral("agent"), QStringLiteral("Request sent"));
    w->updateChatControls();
    w->updateCommandStates();
    return true;
}

QJsonObject ReviewWiring::commentForFocus(const QJsonObject &focus) const {
    const double fps = std::max(1.0, m_window->m_fps);
    if (!focus.contains(QStringLiteral("fromFrame"))) return {};
    const double from = focus.value(QStringLiteral("fromFrame")).toDouble() / fps;
    const double to = focus.value(QStringLiteral("toFrame")).toDouble() / fps;
    QJsonObject comment{{QStringLiteral("at"), from}};
    if (to > from) comment.insert(QStringLiteral("duration"), to - from);
    return comment;
}

bool ReviewWiring::resolveThread(const QString &threadId, bool resolve) {
    const ReviewModel::Thread *thread = m_model->thread(threadId);
    if (!thread || thread->origin != ReviewModel::Origin::Record) return false;
    const auto set = [this](const QString &id, bool resolved) {
        return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("comment_set")},
                                       {QStringLiteral("id"), id},
                                       {QStringLiteral("resolved"), resolved}});
    };
    if (resolve) return set(thread->root.id, true);
    // Reopening: the root, and the comments an accept resolved (it never
    // resolves the root itself).
    bool sent = false;
    if (thread->root.resolved) sent = set(thread->root.id, false) || sent;
    if (!thread->acceptedProposal.isEmpty()) {
        const ReviewModel::Proposal *accepted = m_model->proposal(thread->acceptedProposal);
        QList<const ReviewModel::Comment *> comments{&thread->root};
        for (const ReviewModel::Comment &reply : thread->replies) comments << &reply;
        for (const ReviewModel::Comment *comment : comments)
            if (comment->resolved && !comment->proposal.isEmpty() && accepted && accepted->hasVersion(comment->proposal) &&
                comment->id != thread->root.id)
                sent = set(comment->id, false) || sent;
    }
    return sent;
}

bool ReviewWiring::accept(const QString &proposalId) {
    if (m_model->guestMode()) return false;
    const bool sent = m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")},
                                              {QStringLiteral("proposalId"), proposalId},
                                              {QStringLiteral("revision"), m_window->revisionValue()}});
    if (sent && m_panel) m_panel->markSettledHere(proposalId, true);
    return sent;
}

bool ReviewWiring::reject(const QString &proposalId) {
    if (m_model->guestMode()) return false;
    const bool sent = m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("reject")},
                                              {QStringLiteral("proposalId"), proposalId},
                                              {QStringLiteral("revision"), m_window->revisionValue()}});
    if (sent && m_panel) m_panel->markSettledHere(proposalId, false);
    return sent;
}

bool ReviewWiring::preview(const QString &proposalId, bool on) {
    if (!on) return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview_exit")}});
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("preview")}, {QStringLiteral("proposalId"), proposalId}};
    // Over the proposal's range (at most 120 s), when it has one.
    if (const ReviewModel::Proposal *proposal = m_model->proposal(proposalId); proposal && proposal->hasRange) {
        const double start = std::max(0.0, proposal->start);
        const double length = std::clamp(proposal->end - start, 1.0 / std::max(1.0, m_window->m_fps), 120.0);
        command.insert(QStringLiteral("start"), start);
        command.insert(QStringLiteral("duration"), length);
    }
    return m_bus->send(command);
}

bool ReviewWiring::previewRange(const QString &proposalId, double start, double duration) {
    return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview")},
                                   {QStringLiteral("proposalId"), proposalId},
                                   {QStringLiteral("start"), start},
                                   {QStringLiteral("duration"), duration}});
}

bool ReviewWiring::exportComments(const QString &format, const QString &path) {
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("export_comments")},
                        {QStringLiteral("format"), format},
                        {QStringLiteral("includeResolved"), false}};
    if (!path.isEmpty()) command.insert(QStringLiteral("path"), path);
    return m_bus->send(command);
}

bool ReviewWiring::convertToMarker(const QString &commentId) {
    const ReviewModel::Comment *comment = m_model->comment(commentId);
    if (!comment || !comment->hasAt()) return false;
    QJsonObject op{{QStringLiteral("op"), QStringLiteral("add-marker")},
                   {QStringLiteral("at"), m_window->frameAt(comment->at)},
                   {QStringLiteral("name"), comment->author.name},
                   {QStringLiteral("note"), comment->body}};
    if (comment->duration > 0) op.insert(QStringLiteral("frames"), m_window->frameAt(comment->duration));
    return m_window->sendEdit(QJsonArray{op});
}

bool ReviewWiring::deleteComment(const QString &commentId) {
    return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("comment_set")},
                                   {QStringLiteral("id"), commentId},
                                   {QStringLiteral("deleted"), true}});
}

bool ReviewWiring::editComment(const QString &commentId, const QString &body) {
    return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("comment_set")},
                                   {QStringLiteral("id"), commentId},
                                   {QStringLiteral("body"), body}});
}

QString ReviewWiring::reviewLink(double seconds) const {
    // Named by the folder's slug, the name a link is followed by (not the
    // document's, which a closed project could not be found by).
    const MainWindow *w = m_window;
    return links::make(w->m_projectDirectory, w->m_revision, seconds, w->m_fps);
}

void ReviewWiring::syncPinnedProposal() {
    MainWindow *w = m_window;
    const QList<ReviewModel::Proposal> open = m_model->openProposals();
    if (open.isEmpty() && m_model->proposals().isEmpty()) return; // no proposals list yet: updateProposal's own.
    // The version the pinned card shows: the one its Accept and preview send.
    const QString pinned = open.isEmpty() ? QString()
                           : m_panel      ? m_panel->shownVersion(open.constFirst().id)
                                          : open.constFirst().id;
    const bool stale = !open.isEmpty() && open.constFirst().stale;
    if (pinned == w->m_proposalId && stale == w->m_proposalBlocked) return;
    const bool moved = pinned != w->m_proposalId;
    w->m_proposalId = pinned;
    if (moved && w->m_timeline) {
        w->m_timeline->setProposal(window_parts::proposalTimelineFor(w->m_snapshot, pinned));
        w->showProposedCut();
    }
    w->m_proposalBlocked = stale;
    w->updateCommandStates();
}

void ReviewWiring::handleEvent(const QString &type, const QJsonObject &event) {
    if (type == QLatin1String("comments") || type == QLatin1String("state") || type == QLatin1String("collab")) {
        m_model->applyEvent(type, event);
        if (type == QLatin1String("state") && m_panel) {
            const QJsonObject snapshot = event.value(QStringLiteral("snapshot")).toObject().isEmpty()
                                             ? event.value(QStringLiteral("state")).toObject()
                                             : event.value(QStringLiteral("snapshot")).toObject();
            m_panel->setSnapshot(snapshot);
        }
        if (type == QLatin1String("comments") && m_panel) m_panel->reconcileLocal();
    } else if (type == QLatin1String("connection")) {
        const bool ready = event.value(QStringLiteral("status")).toString() == QLatin1String("ready");
        if (m_panel) m_panel->setConnected(ready);
        if (ready) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("comment_list")}});
    } else if (type == QLatin1String("check")) {
        if (event.value(QStringLiteral("status")).toString() == QLatin1String("done") && m_panel)
            m_panel->setCheckRows(event.value(QStringLiteral("summary")).toObject().value(QStringLiteral("rows")).toArray());
    } else if (type == QLatin1String("preview")) {
        const QString status = event.value(QStringLiteral("status")).toString();
        if (!m_panel) return;
        if (status == QLatin1String("stopped") || status == QLatin1String("exited") || status == QLatin1String("failed"))
            m_panel->setPreviewProposal(QString());
        else if (event.contains(QStringLiteral("proposalId")))
            m_panel->setPreviewProposal(event.value(QStringLiteral("proposalId")).toString());
    } else if (type == QLatin1String("error")) {
        const QString command = event.value(QStringLiteral("command")).toString();
        if (!m_panel || !command.startsWith(QLatin1String("comment_"))) return;
        const QString message = event.value(QStringLiteral("message")).toString(QStringLiteral("The review log refused it"));
        const QString id = event.value(QStringLiteral("id")).toString();
        if (!id.isEmpty()) m_panel->failPendingNote(id, message);
        m_panel->composerWidget()->setError(QStringLiteral("Couldn't save the note · %1").arg(message));
    }
}

// ======================================================================= MainWindow

void MainWindow::askAboutInChat(const QString &subject) {
    // The panel, open, with the subject in the composer.
    QString text = m_chatInput->toPlainText().trimmed();
    if (!text.startsWith(subject)) text = subject + QLatin1Char(' ') + text;
    else text += QLatin1Char(' ');
    m_chatInput->setPlainText(text.endsWith(QLatin1Char(' ')) ? text : text + QLatin1Char(' '));
    QTextCursor cursor = m_chatInput->textCursor();
    cursor.movePosition(QTextCursor::End);
    m_chatInput->setTextCursor(cursor);
    m_chatInput->setFocus(Qt::OtherFocusReason);
}

void MainWindow::sendChat() { m_reviewWiring->post(); }

bool MainWindow::sendAgent(const QString &text, const QJsonObject &focus, const QString &focusLabel) {
    return m_reviewWiring->sendTurn(text, focus, focusLabel, m_reviewWiring->commentForFocus(focus), QString());
}

void MainWindow::askAi() {
    // Anchor to the selected clip, or to the timeline at the playhead.
    const QJsonObject clip = selectedClipObject();
    const QRect bounds = clip.isEmpty() ? QRect() : m_timeline->clipBounds(m_selectedClip);
    const QRect visible = m_timelineScroll->viewport()->rect();
    QPoint anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(visible.center().x(), 48));
    if (!bounds.isEmpty()) {
        const QPoint top = m_timeline->mapTo(m_timelineScroll->viewport(), bounds.topLeft() + QPoint(bounds.width() / 2, 0));
        if (visible.contains(QPoint(std::clamp(top.x(), 0, visible.width() - 1), std::clamp(top.y(), 0, visible.height() - 1))))
            anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(std::clamp(top.x(), 24, visible.width() - 24), std::max(top.y(), 24)));
    }
    const QString title = QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps));
    m_askPrompt->open(anchor, title, clip.isEmpty() ? QString() : clipName(clip));
}

void MainWindow::askSubmitted(const QString &text, const QString &scope) {
    const QJsonObject clip = selectedClipObject();
    QJsonObject focus{{QStringLiteral("scope"), scope}};
    QString label;
    if (!clip.isEmpty()) {
        const qint64 start = clipFrames(clip, QStringLiteral("startFrame"));
        focus.insert(QStringLiteral("clip"), m_selectedClip);
        focus.insert(QStringLiteral("fromFrame"), start);
        focus.insert(QStringLiteral("toFrame"), start + clipFrames(clip, QStringLiteral("durationFrames")));
        label = scope == QStringLiteral("clip") ? clipName(clip) : QStringLiteral("Sequence around %1").arg(clipName(clip));
    } else {
        focus.insert(QStringLiteral("fromFrame"), frameAt(m_playhead));
        focus.insert(QStringLiteral("toFrame"), frameAt(m_playhead));
        label = QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps));
    }
    sendAgent(text, focus, label);
}

void MainWindow::showNextSteps(const QJsonArray &steps) {
    // The welcome's Try asking and the composer's suggestions.
    m_chat->setNextSteps(steps);
}

void MainWindow::updateChatControls() {
    const bool hasText = !m_chatInput->toPlainText().trimmed().isEmpty();
    m_sendButton->setEnabled(m_connected && hasText && !m_agentBusy);
    m_abortButton->setEnabled(m_agentBusy);
    m_chat->updateComposer(m_agentBusy);
    if (m_sidebar) m_sidebar->setWorking(m_agentBusy);
    updateComposerHint();
    m_chat->setConnected(m_connected);
    updateChatStatus();
}

void MainWindow::updateComposerHint() {
    // The design's one placeholder. Offline says so, and so does an agent that
    // cannot take requests (no key, Kerf Cloud signed out): the registry's
    // reason for ai.ask says why and what to do.
    QString placeholder = QStringLiteral("Ask Kerf, or leave a note");
    QString tip = QStringLiteral("Return sends · Shift-Return starts a new line");
    if (!m_connected) {
        placeholder = QStringLiteral("Connect to comment or ask Kerf");
        tip = QStringLiteral("Connect to start editing");
    } else if (const QString reason = m_commands ? m_commands->reason(QStringLiteral("ai.ask")) : QString();
               !m_agentBusy && !reason.isEmpty()) {
        placeholder = QStringLiteral("Comment, or set up Kerf in Settings to ask");
        tip = reason;
    }
    m_chatInput->setPlaceholderText(placeholder);
    m_chatInput->setToolTip(tip);
}

void MainWindow::updateProposal(const QJsonValue &value) {
    // The snapshot's first open proposal; the review model's pin order takes
    // over once the state event reaches it (ReviewWiring::syncPinnedProposal).
    // Its card in the panel and its bar on the timeline show it.
    m_proposalId.clear();
    m_proposalBlocked = false;
    const QJsonObject proposal = value.toObject();
    if (proposal.value(QStringLiteral("status")).toString() == QStringLiteral("pending")) {
        m_proposalId = proposal.value(QStringLiteral("proposalId")).toString();
        m_proposalBlocked = proposal.value(QStringLiteral("freshness")).toString() == QStringLiteral("stale");
        if (m_reviewWiring) m_reviewWiring->syncPinnedProposal();
    }
}

ProposalChange MainWindow::describeScriptLine(const QString &line) const {
    // A script line as a change a reader follows: what it does, to which
    // clip (by its name), and how much or where. Anything not known here
    // reads as the line itself.
    const QStringList words = scriptWords(line);
    ProposalChange change;
    change.description = line.left(1).toUpper() + line.mid(1);
    if (words.isEmpty()) return change;
    const QString verb = words.at(0);
    // The names said, so the short description can drop their extensions.
    QStringList names;
    const auto clipName = [this, &names](const QString &id) {
        const QJsonObject clip = clipObject(id);
        const QString name = clip.value(QStringLiteral("name")).toString();
        names << (name.isEmpty() ? id : name);
        return names.constLast();
    };
    const auto clipSpan = [this](const QString &id) {
        const QJsonObject clip = clipObject(id);
        if (clip.isEmpty()) return QString();
        const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
        return QStringLiteral("%1\u2009\u2013\u2009%2").arg(clockTime(start), clockTime(start + clip.value(QStringLiteral("duration")).toDouble()));
    };
    const auto at = [this, &words](const QString &key, int position = -1) {
        QString value = scriptValue(words, key);
        if (value.isEmpty() && position >= 0 && position < words.size()) value = words.at(position);
        const double seconds = scriptSeconds(value, m_fps);
        return std::isnan(seconds) ? value : clockTime(seconds);
    };
    const QString subject = words.value(1);
    if (verb == QStringLiteral("trim") && words.size() >= 3) {
        const bool end = words.at(2) == QStringLiteral("out");
        change.glyph = ProposalChange::Glyph::Trim;
        change.description = QStringLiteral("Trim %1").arg(clipName(subject));
        // Which edge, and by how much: "end −1.5 s", "start to 0:04".
        const QString edge = end ? QStringLiteral("end") : QStringLiteral("start");
        const double amount = scriptSeconds(words.value(3), m_fps);
        if (!std::isnan(amount)) change.detail = QStringLiteral("%1 %2").arg(edge, signedSeconds(amount));
        else if (!scriptValue(words, QStringLiteral("to")).isEmpty()) change.detail = QStringLiteral("%1 to %2").arg(edge, at(QStringLiteral("to")));
        else change.detail = edge;
    } else if (verb == QStringLiteral("remove") || verb == QStringLiteral("delete") || verb == QStringLiteral("ripple-delete") ||
               verb == QStringLiteral("extract")) {
        change.glyph = ProposalChange::Glyph::Remove;
        change.description = QStringLiteral("Remove %1").arg(clipName(subject));
        change.detail = clipSpan(subject);
    } else if (verb == QStringLiteral("add") && words.size() >= 2) {
        const QString kind = words.at(1);
        QString text = scriptValue(words, QStringLiteral("text"));
        // Without its words in the line, the name the proposal gives it.
        for (const QJsonValue &track : m_snapshot.value(QStringLiteral("proposalTimeline")).toObject().value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
                if (text.isEmpty() && clip.toObject().value(QStringLiteral("id")).toString() == words.value(2))
                    text = clip.toObject().value(QStringLiteral("name")).toString();
        change.glyph = kind == QStringLiteral("title") ? ProposalChange::Glyph::Title : ProposalChange::Glyph::Add;
        change.description = !text.isEmpty() ? QStringLiteral("Add %1 “%2”").arg(kind, text) : QStringLiteral("Add a %1").arg(kind);
        const int atWord = int(words.indexOf(QStringLiteral("at")));
        const QString where = atWord >= 0 ? at(QString(), atWord + 1) : at(QStringLiteral("at"));
        if (!where.isEmpty()) change.detail = QStringLiteral("at %1").arg(where);
    } else if (verb == QStringLiteral("insert") || verb == QStringLiteral("overwrite")) {
        change.glyph = ProposalChange::Glyph::Add;
        const QString media = scriptValue(words, QStringLiteral("media"));
        QString name = media;
        for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
            if (value.toObject().value(QStringLiteral("id")).toString() == media) name = sourceName(value.toObject());
        names << name;
        change.description = QStringLiteral("%1 %2").arg(verb == QStringLiteral("insert") ? QStringLiteral("Insert") : QStringLiteral("Overwrite with"), name);
        if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("at %1").arg(at(QStringLiteral("at")));
    } else if ((verb == QStringLiteral("move") || verb == QStringLiteral("roll") || verb == QStringLiteral("slide") ||
                verb == QStringLiteral("slip")) && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Move;
        change.description = QStringLiteral("%1 %2").arg(verb.left(1).toUpper() + verb.mid(1), clipName(subject));
        const double amount = scriptSeconds(words.value(2), m_fps);
        if (!std::isnan(amount)) change.detail = signedSeconds(amount);
        else if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("to %1").arg(at(QStringLiteral("at")));
        else if (!scriptValue(words, QStringLiteral("to")).isEmpty()) change.detail = QStringLiteral("to %1").arg(at(QStringLiteral("to")));
    } else if (verb == QStringLiteral("split") && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Split;
        change.description = QStringLiteral("Split %1").arg(clipName(subject));
        if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("at %1").arg(at(QStringLiteral("at")));
    } else if (verb == QStringLiteral("set") && words.size() >= 3) {
        QStringList keys;
        for (const QString &word : words.mid(2)) keys << word.section(QLatin1Char('='), 0, 0);
        const QString gain = scriptValue(words, QStringLiteral("gain"));
        if (!gain.isEmpty() || keys.contains(QStringLiteral("volume"))) {
            change.glyph = ProposalChange::Glyph::Level;
            change.description = QStringLiteral("Change %1’s level").arg(clipName(subject));
            bool ok = false;
            const double db = gain.toDouble(&ok);
            if (ok) change.detail = QStringLiteral("%1%2 dB").arg(db < 0 ? QStringLiteral("\u2212") : QString(), QString::number(std::abs(db), 'g', 3));
        } else {
            change.glyph = ProposalChange::Glyph::Other;
            change.description = QStringLiteral("Change %1’s %2").arg(clipName(subject), keys.join(QStringLiteral(", ")));
        }
    } else if (verb == QStringLiteral("fx") && words.size() >= 3) {
        change.glyph = ProposalChange::Glyph::Effect;
        const QString effect = words.value(3).section(QLatin1Char('.'), -1);
        change.description = words.at(2) == QStringLiteral("add") ? QStringLiteral("Add %1 to %2").arg(effect, clipName(subject))
                           : words.at(2) == QStringLiteral("remove") ? QStringLiteral("Remove an effect from %1").arg(clipName(subject))
                                                                       : QStringLiteral("Change the effects on %1").arg(clipName(subject));
    } else if (verb == QStringLiteral("mask") && words.size() >= 3) {
        change.glyph = ProposalChange::Glyph::Effect;
        change.description = words.at(2) == QStringLiteral("add") ? QStringLiteral("Add a mask to %1").arg(clipName(subject))
                                                                   : QStringLiteral("Change the masks on %1").arg(clipName(subject));
    } else if (verb == QStringLiteral("transition")) {
        change.glyph = ProposalChange::Glyph::Add;
        const QString kind = scriptValue(words, QStringLiteral("fx"));
        change.description = QStringLiteral("Add a %1").arg(kind.isEmpty() ? QStringLiteral("transition") : kind.section(QLatin1Char('.'), -1));
        const double length = scriptSeconds(scriptValue(words, QStringLiteral("dur")), m_fps);
        if (!std::isnan(length)) change.detail = signedSeconds(length).mid(1);
    } else if (verb == QStringLiteral("captions") && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Title;
        change.description = QStringLiteral("Caption %1").arg(clipName(subject));
    }
    // For a narrow row: "Trim fisher-close" (the tooltip keeps the full name).
    change.shortDescription = change.description;
    // Only a media file's own extension goes: names from media come without
    // one already (see sourceName), and "clip.v2" keeps its ".v2".
    static const QStringList mediaSuffixes{
        QStringLiteral("mov"), QStringLiteral("mp4"), QStringLiteral("m4v"), QStringLiteral("mkv"), QStringLiteral("webm"),
        QStringLiteral("avi"), QStringLiteral("mxf"), QStringLiteral("wav"), QStringLiteral("mp3"), QStringLiteral("m4a"),
        QStringLiteral("aac"), QStringLiteral("aif"), QStringLiteral("aiff"), QStringLiteral("flac"), QStringLiteral("png"),
        QStringLiteral("jpg"), QStringLiteral("jpeg"), QStringLiteral("gif"), QStringLiteral("tif"), QStringLiteral("tiff")};
    for (const QString &name : std::as_const(names)) {
        const QFileInfo file(name);
        const QString base = file.completeBaseName();
        if (base.isEmpty() || !mediaSuffixes.contains(file.suffix(), Qt::CaseInsensitive)) continue;
        change.shortDescription.replace(name, base);
    }
    return change;
}


void MainWindow::appendConversation(const QString &speaker, const QString &text) {
    if (text.isEmpty()) return;
    const bool user = speaker == QStringLiteral("You");
    const bool error = speaker == QStringLiteral("Error");
    if (user) {
        // A new turn: Kerf's steps from now on belong to its next reply.
        m_chat->beginTurn(text, QString());
        m_agentRow = nullptr;
        m_pendingReplies.clear();
    } else if (error) {
        m_chat->addComment(CommentRow::Kind::Error, text);
    } else {
        CommentRow *row = m_chat->turnKerfRow(false);
        if (row && row->isDrafting()) {
            row->setText(text);
        } else {
            row = m_chat->addComment(CommentRow::Kind::Kerf, text);
        }
        m_agentRow = row;
        for (const QString &reply : std::as_const(m_pendingReplies)) row->addReply(reply);
        m_pendingReplies.clear();
    }
}

void MainWindow::showActivity() {
    QWidget *more = m_chat->moreButton();
    m_activityPopup->adjustSize();
    m_activityPopup->move(more->mapToGlobal(QPoint(more->width() - m_activityPopup->width(), more->height() + 4)));
    m_activityPopup->show();
}

void MainWindow::updateChatStatus() {
    // Kerf at work shows in its drafting row: the latest status line.
    if (m_agentBusy && !m_activityDetail.isEmpty()) m_chat->setTurnStatus(m_activityDetail);
}

void MainWindow::appendConversationNote(const QString &kind, const QString &text) {
    if (text.isEmpty()) return;
    if (kind == QStringLiteral("Agent error")) {
        appendConversation(QStringLiteral("Error"), text);
        return;
    }
    const QString detail = text.simplified();
    // A tool's work is a step of Kerf's row for this turn, folded behind
    // "2 steps"; until Kerf's row exists, it waits.
    if (kind == QStringLiteral("Tool")) {
        if (m_agentRow) m_agentRow->addReply(detail);
        else m_pendingReplies.append(detail);
    }
    m_activityDetail = detail;
    m_chat->setTurnStatus(detail);
    updateChatStatus();
}

bool MainWindow::handleAgentEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type == QStringLiteral("agent")) {
        const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
        const QString text = event.value(QStringLiteral("text")).toString();
        if (kind == QStringLiteral("delta")) {
            // The stream shows in the turn's drafting row until done.
            CommentRow *row = m_chat->turnKerfRow(true);
            if (m_agentText.isEmpty()) row->setText(text);
            else row->appendText(text);
            for (const QString &reply : std::as_const(m_pendingReplies)) row->addReply(reply);
            m_pendingReplies.clear();
            m_agentRow = row;
            m_agentText += text;
        } else if (kind == QStringLiteral("done")) {
            CommentRow *row = m_chat->turnKerfRow(false);
            if (m_agentText.isEmpty() && !text.isEmpty()) {
                if (row) row->setText(text);
                else row = m_chat->addComment(CommentRow::Kind::Kerf, text);
            }
            if (row) {
                for (const QString &reply : std::as_const(m_pendingReplies)) row->addReply(reply);
                m_pendingReplies.clear();
            }
            m_agentText.clear();
            m_chat->finishTurn();
            m_agentRow = nullptr;
            appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Response complete"));
        } else if (kind == QStringLiteral("tool")) {
            appendConversationNote(QStringLiteral("Tool"), text);
            addActivity(QStringLiteral("tool"), text.left(120));
            if (text.startsWith(QStringLiteral("render"), Qt::CaseInsensitive)) m_turnRendered = true;
        } else if (kind == QStringLiteral("status")) {
            appendConversationNote(QStringLiteral("Progress"), text);
            addActivity(QStringLiteral("status"), text.left(120));
        } else {
            appendConversationNote(QStringLiteral("Agent"), text);
            addActivity(kind, text.left(120));
        }
    } else if (type == QStringLiteral("agent_busy")) {
        const bool busy = event.value(QStringLiteral("busy")).toBool();
        m_agentBusy = busy;
        m_chat->setTurnBusy(busy);
        if (busy) showSheet();
        quietenCutPreview();
        if (!busy) {
            m_chat->finishTurn();
            endAgentFollow();
            // What Kerf was asked to make is made: it goes home.
            if (m_turnRendered) goHome();
            m_turnRendered = false;
        }
        updateChatControls();
            updateCommandStates();
    } else if (type == QStringLiteral("agent_state")) {
        const QString state = event.value(QStringLiteral("state")).toString();
        updateChatStatus();
        if (state == QStringLiteral("failed")) {
            appendConversationNote(QStringLiteral("Agent error"), event.value(QStringLiteral("message")).toString());
            m_chat->finishTurn();
        } else if (state == QStringLiteral("aborted")) {
            // "Kerf · stopped", with Ask again.
            m_chat->stopTurn();
        }
    } else if (type == QStringLiteral("agent_next")) {
        showNextSteps(event.value(QStringLiteral("steps")).toArray());
    } else if (type == QStringLiteral("agent_focus")) {
        followAgentFocus(event);
    } else {
        return false;
    }
    return true;
}

// The comments panel (the agent chat).
void MainWindow::buildReview() {
    m_chat = new ChatPanel;
    m_chatScroll = m_chat->scroll();
    m_chatComposer = m_chat->composer();
    m_chatInput = m_chat->input();
    m_sendButton = m_chat->sendButton();
    m_abortButton = m_chat->stopButton();
    connect(m_chatInput, &QTextEdit::textChanged, this, &MainWindow::updateChatControls);
    // Stop (m_abortButton) reaches kerfd through ChatPanel::stopRequested (ReviewWiring).
    m_abortButton->setEnabled(false);
    // What the agent and the project did, one line each, from More.
    m_activityPopup = new QFrame(this, Qt::Popup);
    m_activityPopup->setObjectName(QStringLiteral("activityPopup"));
    {
        auto *layout = new QVBoxLayout(m_activityPopup);
        layout->setContentsMargins(12, 10, 12, 10);
        layout->setSpacing(6);
        auto *title = new QLabel(QStringLiteral("Activity"));
        title->setObjectName(QStringLiteral("eyebrow"));
        layout->addWidget(title);
        m_activityList = new QListWidget;
        m_activityList->setObjectName(QStringLiteral("activityList"));
        m_activityList->setMinimumSize(320, 160);
        layout->addWidget(m_activityList);
    }
    m_reviewWiring->attach(m_chat);
}

} // namespace kerf
