#include "ask_prompt.h"

#include <QApplication>
#include <QComboBox>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QPushButton>
#include <QScreen>
#include <QTextEdit>
#include <QVBoxLayout>

#include <algorithm>

namespace cut {

AskPrompt::AskPrompt(QWidget *parent) : QFrame(parent, Qt::Popup) {
    setObjectName(QStringLiteral("askPrompt"));
    setAttribute(Qt::WA_StyledBackground);
    setFixedWidth(380);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(12, 10, 12, 10);
    layout->setSpacing(8);
    m_title = new QLabel;
    m_title->setObjectName(QStringLiteral("askTitle"));
    layout->addWidget(m_title);
    m_input = new QTextEdit;
    m_input->setObjectName(QStringLiteral("askInput"));
    m_input->setAcceptRichText(false);
    m_input->setPlaceholderText(QStringLiteral("Tell Cut what to change…"));
    m_input->setFixedHeight(64);
    m_input->setTabChangesFocus(true);
    m_input->installEventFilter(this);
    layout->addWidget(m_input);
    auto *row = new QHBoxLayout;
    row->setSpacing(8);
    m_scope = new QComboBox;
    m_scope->setObjectName(QStringLiteral("askScope"));
    row->addWidget(m_scope);
    row->addStretch(1);
    auto *hint = new QLabel(QStringLiteral("↵ Send  ·  ⇧↵ New line"));
    hint->setObjectName(QStringLiteral("muted"));
    row->addWidget(hint);
    auto *send = new QPushButton(QStringLiteral("Ask"));
    send->setObjectName(QStringLiteral("primaryButton"));
    connect(send, &QPushButton::clicked, this, &AskPrompt::submit);
    row->addWidget(send);
    layout->addLayout(row);
}

void AskPrompt::open(const QPoint &anchor, const QString &title, bool hasClip) {
    m_title->setText(QStringLiteral("✦  %1").arg(title));
    m_scope->clear();
    if (hasClip) m_scope->addItem(QStringLiteral("This clip"), QStringLiteral("clip"));
    m_scope->addItem(QStringLiteral("Whole sequence"), QStringLiteral("sequence"));
    m_input->clear();
    adjustSize();
    // Above the anchor, kept on its screen.
    QPoint at(anchor.x() - width() / 2, anchor.y() - height() - 8);
    if (const QScreen *screen = QApplication::screenAt(anchor)) {
        const QRect area = screen->availableGeometry();
        at.setX(std::clamp(at.x(), area.left() + 8, area.right() - width() - 8));
        if (at.y() < area.top() + 8) at.setY(anchor.y() + 16);
    }
    move(at);
    show();
    m_input->setFocus();
}

QString AskPrompt::scope() const {
    return m_scope->currentData().toString();
}

bool AskPrompt::eventFilter(QObject *object, QEvent *event) {
    if (object == m_input && event->type() == QEvent::KeyPress) {
        const auto *key = static_cast<QKeyEvent *>(event);
        if ((key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) && !(key->modifiers() & Qt::ShiftModifier)) {
            submit();
            return true;
        }
    }
    return QFrame::eventFilter(object, event);
}

void AskPrompt::submit() {
    const QString text = m_input->toPlainText().trimmed();
    if (text.isEmpty()) return;
    hide();
    emit submitted(text, scope());
}

} // namespace cut
