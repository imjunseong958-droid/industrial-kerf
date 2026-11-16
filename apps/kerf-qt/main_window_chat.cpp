// MainWindow's conversation panel, proposals, Ask and next steps, and
// ChatWiring's members that reach into the window.
//
// The conversation is kept beside the project (.kerf/conversations): the
// bridge records each agent turn, the request and then Kerf's answer with
// its steps, and sends the conversation again. While a turn runs, the panel
// shows it as live messages (the request, then Kerf at work with the
// stream's latest status) until the kept turn takes their place.

#include "main_window.h"
#include "project_sidebar.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "chat_parts.h"
#include "components.h"
#include "composer.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "proposal_card.h"
#include "proposal_model.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "stage_shell.h"
#include "wiring/main_window_shared.h"
#include "ui/conversation_model.h"
#include "wiring/chat_wiring.h"

#include <QApplication>
#include <QClipboard>
#include <QDateTime>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QJsonDocument>
#include <QLabel>
#include <QListWidget>
#include <QMessageBox>
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

// ======================================================================= ChatWiring

void ChatWiring::attach(ChatPanel *panel) {
    m_panel = panel;
    MainWindow *w = m_window;
    m_bus->setProposals(m_model);
    panel->setProposals(m_model);
    panel->setConversations(m_conversations);
    panel->setScriptDescriber([w](const QString &line) { return w->describeScriptLine(line); });

    // The bus feeds the models and the panel.
    connect(m_bus, &EditorBus::event, this, &ChatWiring::handleEvent);
    connect(m_bus, &EditorBus::playheadChanged, this, [this](double seconds) { m_model->setPlayhead(seconds); });
    connect(m_bus, &EditorBus::snapshotChanged, this, [this] {
        if (!m_panel) return;
        m_panel->setFps(m_bus->fps());
        m_panel->setNamedTimeline(m_bus->timeline());
    });
    // Cross-region requests.
    connect(m_bus, &EditorBus::askPrefill, this, [this](const QString &text) {
        if (!m_panel) return;
        // Filled where it can be seen.
        m_window->uncoverComposer();
        m_panel->composerWidget()->fill(text);
    });
    connect(m_model, &ProposalModel::changed, this, &ChatWiring::syncPinnedProposal);
    connect(m_model, &ProposalModel::changed, m_bus, &EditorBus::proposalsChanged);

    // What the panel asks for.
    connect(panel, &ChatPanel::postRequested, this, &ChatWiring::post);
    connect(panel, &ChatPanel::seekRequested, w, &MainWindow::queueSeek);
    connect(panel, &ChatPanel::acceptRequested, this, &ChatWiring::accept);
    connect(panel, &ChatPanel::rejectRequested, this, &ChatWiring::reject);
    connect(panel, &ChatPanel::previewRequested, this, &ChatWiring::preview);
    // ⌘↩ and Esc act on the version the pinned card shows.
    connect(panel, &ChatPanel::shownVersionChanged, this, &ChatWiring::syncPinnedProposal);
    // The composer's Stop and Esc while busy: one abort each.
    connect(panel, &ChatPanel::stopRequested, w, [w] {
        if (!w->execute(QStringLiteral("ai.stop"))) return;
        w->m_abortButton->setEnabled(false);
        w->m_chat->setTurnStatus(QStringLiteral("Stopping…"));
    });
    connect(panel, &ChatPanel::runRequested, this, [this](const QString &what) {
        if (what == QLatin1String("qc")) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("check")}});
    });
    connect(panel, &ChatPanel::askAgain, this, [this, w](const QString &text) {
        if (w->m_agentBusy || !w->m_connected || text.trimmed().isEmpty()) return;
        sendTurn(text, w->m_lastFocus, w->m_lastFocusLabel);
    });
    connect(panel, &ChatPanel::conversationAction, this, [this, w](const QString &action, const QString &argument) {
        const QString id = m_conversations->currentId();
        if (id.isEmpty()) return;
        if (action == QLatin1String("rename")) {
            renameConversation(id, argument);
        } else if (action == QLatin1String("archive")) {
            archiveConversation(id, argument != QLatin1String("false"));
        } else if (action == QLatin1String("delete")) {
            if (w->confirmDeleteConversation(m_conversations->current().summary.title)) deleteConversation(id);
        }
    });
    connect(panel, &ChatPanel::moreAction, this, [this, w](const QString &action, const QString &argument) {
        if (action == QLatin1String("settings")) {
            w->showSettings(QStringLiteral("ai"));
        } else if (action == QLatin1String("activity")) {
            w->showActivity();
        } else if (action == QLatin1String("model")) {
            w->m_agentModel = argument;
            m_panel->setModels(w->m_agentModels, argument, true);
        }
    });

    panel->setFps(m_bus->fps());
}

bool ChatWiring::post() {
    MainWindow *w = m_window;
    if (!m_panel) return false;
    Composer *composer = m_panel->composerWidget();
    const QString text = composer->text().trimmed();
    if (text.isEmpty() || !w->m_connected || w->m_agentBusy) return false;
    const bool stamped = composer->stamped();
    const double at = w->m_playhead;
    const QJsonObject focus{{QStringLiteral("scope"), QStringLiteral("sequence")},
                            {QStringLiteral("fromFrame"), w->frameAt(at)},
                            {QStringLiteral("toFrame"), w->frameAt(at)}};
    const QString label = QStringLiteral("Sequence at %1").arg(timelineTimecode(at, w->m_fps));
    if (!sendTurn(text, focus, label, stamped ? at : std::numeric_limits<double>::quiet_NaN())) return false;
    composer->clear();
    w->updateChatControls();
    return true;
}

bool ChatWiring::sendTurn(const QString &text, const QJsonObject &focus, const QString &focusLabel, double at) {
    MainWindow *w = m_window;
    QJsonObject request = w->agentSettingsPayload();
    request.insert(QStringLiteral("type"), QStringLiteral("agent"));
    request.insert(QStringLiteral("text"), text);
    if (!focus.isEmpty()) request.insert(QStringLiteral("focus"), focus);
    // The bridge keeps the turn in the open conversation, or starts one.
    const QString conversation = m_conversations->currentId();
    if (!conversation.isEmpty()) request.insert(QStringLiteral("conversation"), conversation);
    if (std::isfinite(at)) request.insert(QStringLiteral("at"), at);
    // The AI bar names what Kerf works on once the bridge reports it busy.
    w->m_agentFocusLabel = focusLabel;
    if (!m_bus->send(request)) return false;
    if (conversation.isEmpty()) m_conversations->expectNew();
    w->m_lastFocus = focus;
    w->m_lastFocusLabel = focusLabel;
    w->showNextSteps(QJsonArray{});
    w->m_agentBusy = true;
    w->m_agentText.clear();
    w->m_abortButton->setEnabled(true);
    if (m_panel) {
        m_panel->beginTurn(text, at);
        m_panel->setTurnBusy(true);
        m_panel->setTurnStatus(focusLabel);
    }
    w->addActivity(QStringLiteral("agent"), QStringLiteral("Request sent"));
    w->updateChatControls();
    w->updateCommandStates();
    return true;
}

bool ChatWiring::sendConversations(const QJsonObject &command) {
    QJsonObject sent = command;
    sent.insert(QStringLiteral("type"), QStringLiteral("conversations"));
    return m_bus->send(sent);
}

void ChatWiring::openConversation(const QString &id) {
    if (id == m_conversations->currentId() && !id.isEmpty()) return;
    m_conversations->choose(id);
    if (!id.isEmpty()) sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("open")}, {QStringLiteral("id"), id}});
    if (m_panel) m_panel->focusComposer();
}

bool ChatWiring::archiveConversation(const QString &id, bool archived) {
    return sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("archive")},
                                         {QStringLiteral("id"), id},
                                         {QStringLiteral("archived"), archived}});
}

bool ChatWiring::renameConversation(const QString &id, const QString &title) {
    if (title.trimmed().isEmpty()) return false;
    return sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("rename")},
                                         {QStringLiteral("id"), id},
                                         {QStringLiteral("title"), title.trimmed()}});
}

bool ChatWiring::deleteConversation(const QString &id) {
    const bool sent = sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("delete")}, {QStringLiteral("id"), id}});
    // The one open goes at once: a new conversation in its place.
    if (sent && id == m_conversations->currentId()) m_conversations->choose(QString());
    return sent;
}

bool ChatWiring::accept(const QString &proposalId) {
    const bool sent = m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("accept")},
                                              {QStringLiteral("proposalId"), proposalId},
                                              {QStringLiteral("revision"), m_window->revisionValue()}});
    if (sent && m_panel) m_panel->markSettledHere(proposalId, true);
    return sent;
}

bool ChatWiring::reject(const QString &proposalId) {
    const bool sent = m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("reject")},
                                              {QStringLiteral("proposalId"), proposalId},
                                              {QStringLiteral("revision"), m_window->revisionValue()}});
    if (sent && m_panel) m_panel->markSettledHere(proposalId, false);
    return sent;
}

bool ChatWiring::preview(const QString &proposalId, bool on) {
    if (!on) return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview_exit")}});
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("preview")}, {QStringLiteral("proposalId"), proposalId}};
    // Over the proposal's range (at most 120 s), when it has one.
    if (const ProposalModel::Proposal *proposal = m_model->proposal(proposalId); proposal && proposal->hasRange) {
        const double start = std::max(0.0, proposal->start);
        const double length = std::clamp(proposal->end - start, 1.0 / std::max(1.0, m_window->m_fps), 120.0);
        command.insert(QStringLiteral("start"), start);
        command.insert(QStringLiteral("duration"), length);
    }
    return m_bus->send(command);
}

bool ChatWiring::previewRange(const QString &proposalId, double start, double duration) {
    return m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview")},
                                   {QStringLiteral("proposalId"), proposalId},
                                   {QStringLiteral("start"), start},
                                   {QStringLiteral("duration"), duration}});
}

void ChatWiring::syncPinnedProposal() {
    MainWindow *w = m_window;
    const QList<ProposalModel::Proposal> open = m_model->openProposals();
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

void ChatWiring::handleEvent(const QString &type, const QJsonObject &event) {
    if (type == QLatin1String("state")) {
        m_model->applyEvent(type, event);
        if (m_panel) {
            const QJsonObject snapshot = event.value(QStringLiteral("snapshot")).toObject().isEmpty()
                                             ? event.value(QStringLiteral("state")).toObject()
                                             : event.value(QStringLiteral("snapshot")).toObject();
            m_panel->setSnapshot(snapshot);
        }
    } else if (type == QLatin1String("conversations") || type == QLatin1String("conversation")) {
        m_conversations->applyEvent(type, event);
    } else if (type == QLatin1String("connection")) {
        const bool ready = event.value(QStringLiteral("status")).toString() == QLatin1String("ready");
        if (m_panel) m_panel->setConnected(ready);
        if (!ready) return;
        // The conversations kept beside the project, and the open one again.
        sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("list")}});
        if (const QString id = m_conversations->currentId(); !id.isEmpty())
            sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("open")}, {QStringLiteral("id"), id}});
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
        if (!m_panel || event.value(QStringLiteral("command")).toString() != QLatin1String("conversation")) return;
        m_window->showStatus(QStringLiteral("Couldn't change the conversation · %1")
                                 .arg(event.value(QStringLiteral("message")).toString()),
                             6000);
        // What was shown at once goes back to what is kept.
        sendConversations(QJsonObject{{QStringLiteral("action"), QStringLiteral("list")}});
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

void MainWindow::sendChat() { m_chatWiring->post(); }

bool MainWindow::sendAgent(const QString &text, const QJsonObject &focus, const QString &focusLabel) {
    // Asked about a range: the turn is stamped at its start.
    const double fps = std::max(1.0, m_fps);
    const double at = focus.contains(QStringLiteral("fromFrame")) ? focus.value(QStringLiteral("fromFrame")).toDouble() / fps
                                                                  : std::numeric_limits<double>::quiet_NaN();
    return m_chatWiring->sendTurn(text, focus, focusLabel, at);
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
    QString placeholder = QStringLiteral("Ask Kerf to make or change something");
    QString tip = QStringLiteral("Return sends · Shift-Return starts a new line");
    if (!m_connected) {
        placeholder = QStringLiteral("Connect to ask Kerf");
        tip = QStringLiteral("Connect to start editing");
    } else if (const QString reason = m_commands ? m_commands->reason(QStringLiteral("ai.ask")) : QString();
               !m_agentBusy && !reason.isEmpty()) {
        placeholder = QStringLiteral("Set up Kerf in Settings to ask");
        tip = reason;
    }
    m_chatInput->setPlaceholderText(placeholder);
    m_chatInput->setToolTip(tip);
}

void MainWindow::updateProposal(const QJsonValue &value) {
    // The snapshot's first open proposal; the review model's pin order takes
    // over once the state event reaches it (ChatWiring::syncPinnedProposal).
    // Its card in the panel and its bar on the timeline show it.
    m_proposalId.clear();
    m_proposalBlocked = false;
    const QJsonObject proposal = value.toObject();
    if (proposal.value(QStringLiteral("status")).toString() == QStringLiteral("pending")) {
        m_proposalId = proposal.value(QStringLiteral("proposalId")).toString();
        m_proposalBlocked = proposal.value(QStringLiteral("freshness")).toString() == QStringLiteral("stale");
        if (m_chatWiring) m_chatWiring->syncPinnedProposal();
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
    if (speaker == QStringLiteral("You")) m_chat->beginTurn(text);
    else if (speaker == QStringLiteral("Error")) m_chat->failTurn(text);
    else m_chat->setAnswer(text);
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
    // A tool's work is a step of the turn's work log.
    if (kind == QStringLiteral("Tool")) m_chat->addStep(detail);
    m_activityDetail = detail;
    updateChatStatus();
}

bool MainWindow::handleAgentEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type == QStringLiteral("agent")) {
        const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
        const QString text = event.value(QStringLiteral("text")).toString();
        if (kind == QStringLiteral("delta")) {
            // The stream shows in the turn's live answer until done.
            m_chat->appendAnswer(text);
            m_agentText += text;
        } else if (kind == QStringLiteral("done")) {
            if (m_agentText.isEmpty() && !text.isEmpty()) m_chat->setAnswer(text);
            m_agentText.clear();
            m_chat->finishTurn();
        } else if (kind == QStringLiteral("tool")) {
            appendConversationNote(QStringLiteral("Tool"), text);
            addActivity(QStringLiteral("tool"), text.left(120));
            if (text.startsWith(QStringLiteral("render"), Qt::CaseInsensitive)) m_turnRendered = true;
        } else if (kind == QStringLiteral("status")) {
            m_activityDetail = text.simplified();
            updateChatStatus();
            addActivity(QStringLiteral("status"), text.left(120));
        } else {
            appendConversationNote(QStringLiteral("Agent"), text);
            addActivity(kind, text.left(120));
        }
    } else if (type == QStringLiteral("agent_busy")) {
        const bool busy = event.value(QStringLiteral("busy")).toBool();
        m_agentBusy = busy;
        m_chat->setTurnBusy(busy);
        refreshSidebar();
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
        // What a failed turn said in the banner is stale once another starts.
        if (state == QStringLiteral("running") && m_errorFromAgent) {
            m_errorFromAgent = false;
            m_errorBanner->hide();
        }
        if (state == QStringLiteral("failed")) {
            m_chat->failTurn(event.value(QStringLiteral("message")).toString());
        } else if (state == QStringLiteral("aborted")) {
            // Stopped, with Ask again.
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

// The conversation panel.
void MainWindow::buildChat() {
    m_chat = new ChatPanel;
    m_chatScroll = m_chat->scroll();
    m_chatComposer = m_chat->composer();
    m_chatInput = m_chat->input();
    m_sendButton = m_chat->sendButton();
    m_abortButton = m_chat->stopButton();
    connect(m_chatInput, &QTextEdit::textChanged, this, &MainWindow::updateChatControls);
    // Stop (m_abortButton) reaches kerfd through ChatPanel::stopRequested (ChatWiring).
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
    m_chatWiring->attach(m_chat);
}

bool MainWindow::confirmDeleteConversation(const QString &title) {
    QMessageBox box(QMessageBox::Warning,
                    QStringLiteral("Delete “%1”?").arg(title.isEmpty() ? QStringLiteral("this conversation") : title),
                    QStringLiteral("Its messages are removed from this project. This can't be undone."), QMessageBox::Cancel, this);
    QPushButton *remove = box.addButton(QStringLiteral("Delete"), QMessageBox::DestructiveRole);
    box.setDefaultButton(QMessageBox::Cancel);
    box.exec();
    return box.clickedButton() == remove;
}

} // namespace kerf
