#include "ask_prompt.h"

#include "components.h"
#include "design_tokens.h"

#include <QApplication>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QMenu>
#include <QPainter>
#include <QScreen>
#include <QTextEdit>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

using namespace tokens;

namespace {

// The field: 56 tall, radius 6, raised, an inset line2 (accent in focus).
class AskField final : public QWidget {
public:
    explicit AskField(QTextEdit *input) : m_input(input) {}

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
        ui::paintInsetLine(p, QRectF(rect()), radius::control, color(m_input->hasFocus() ? accent : line2));
    }

private:
    QTextEdit *m_input;
};

QLabel *makeLabel(const TypeStyle &style, QRgb ink) {
    auto *label = new QLabel;
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
    label->setFixedHeight(style.line);
    return label;
}

} // namespace

AskPrompt::AskPrompt(QWidget *parent) : QFrame(parent, Qt::Popup | Qt::FramelessWindowHint | Qt::NoDropShadowWindowHint) {
    setObjectName(QStringLiteral("askPrompt"));
    setAttribute(Qt::WA_TranslucentBackground);
    setStyleSheet(QStringLiteral("QFrame#askPrompt { background: transparent; border: 0; }"));
    const QMargins shadow = ui::shadowMargins();
    setFixedWidth(kWidth + shadow.left() + shadow.right());
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(shadow.left() + 12, shadow.top() + 12, shadow.right() + 12, shadow.bottom() + 12);
    layout->setSpacing(8);
    m_title = makeLabel(bodyName, ink);
    m_title->setObjectName(QStringLiteral("askTitle"));
    layout->addWidget(m_title);

    auto *scopeRow = new QHBoxLayout;
    scopeRow->setSpacing(8);
    QLabel *scope = makeLabel(control, ink2);
    scope->setText(QStringLiteral("Scope"));
    scopeRow->addWidget(scope);
    m_scopeButton = new ui::GhostButton(QStringLiteral("Whole sequence"));
    m_scopeButton->setObjectName(QStringLiteral("askScope"));
    m_scopeButton->setChevron(true);
    m_scopeButton->setTip(QStringLiteral("What Kerf looks at"));
    connect(m_scopeButton, &QAbstractButton::clicked, this, &AskPrompt::showScopeMenu);
    scopeRow->addWidget(m_scopeButton);
    scopeRow->addStretch(1);
    layout->addLayout(scopeRow);

    m_input = new QTextEdit;
    m_input->setObjectName(QStringLiteral("askInput"));
    m_input->setAcceptRichText(false);
    m_input->setFrameShape(QFrame::NoFrame);
    m_input->setTabChangesFocus(true);
    m_input->setFont(tokens::font(tokens::body));
    m_input->document()->setDocumentMargin(0);
    m_input->setPlaceholderText(QStringLiteral("Tell Kerf what to change"));
    m_input->setStyleSheet(QStringLiteral("QTextEdit#askInput, QTextEdit#askInput:focus, QTextEdit#askInput:hover { background: transparent; "
                                          "border: 0; padding: 0; color: %1; font-size: 13px; }")
                               .arg(color(ink).name(QColor::HexRgb)));
    {
        QPalette palette = m_input->palette();
        palette.setColor(QPalette::PlaceholderText, color(ink3));
        m_input->setPalette(palette);
    }
    m_input->installEventFilter(this);
    m_field = new AskField(m_input);
    m_field->setFixedHeight(56);
    auto *fieldLayout = new QVBoxLayout(m_field);
    fieldLayout->setContentsMargins(8, 6, 8, 6);
    fieldLayout->addWidget(m_input);
    layout->addWidget(m_field);

    auto *row = new QHBoxLayout;
    row->setSpacing(8);
    QLabel *hint = makeLabel(meta, ink3);
    hint->setText(QStringLiteral("↵ Send · ⇧↵ New line"));
    row->addWidget(hint);
    row->addStretch(1);
    m_ask = new ui::Button(QStringLiteral("Ask"), ui::Button::Kind::Primary);
    m_ask->setObjectName(QStringLiteral("askSend"));
    m_ask->setTip(QStringLiteral("Ask"), QStringLiteral("↩"));
    connect(m_ask, &QAbstractButton::clicked, this, &AskPrompt::submit);
    row->addWidget(m_ask);
    layout->addLayout(row);
}

QRect AskPrompt::cardRect() const { return rect().marginsRemoved(ui::shadowMargins()); }

void AskPrompt::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF card(cardRect());
    ui::paintShadow(p, card, radius::card);
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::card));
    p.drawRoundedRect(card, radius::card, radius::card);
    ui::paintInsetLine(p, card, radius::card, color(line2));
}

void AskPrompt::setScope(const QString &scope) {
    m_scope = scope == QLatin1String("clip") && !m_clip.isEmpty() ? QStringLiteral("clip") : QStringLiteral("sequence");
    m_scopeButton->setText(m_scope == QLatin1String("clip") ? QStringLiteral("This clip · %1").arg(m_clip) : QStringLiteral("Whole sequence"));
    m_scopeButton->setFixedSize(m_scopeButton->sizeHint());
    m_scopeButton->setEnabled(!m_clip.isEmpty());
}

void AskPrompt::showScopeMenu() {
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    if (!m_clip.isEmpty()) {
        QAction *clip = menu->addAction(QStringLiteral("This clip · %1").arg(m_clip));
        clip->setCheckable(true);
        clip->setChecked(m_scope == QLatin1String("clip"));
        connect(clip, &QAction::triggered, this, [this] { setScope(QStringLiteral("clip")); });
    }
    QAction *sequence = menu->addAction(QStringLiteral("Whole sequence"));
    sequence->setCheckable(true);
    sequence->setChecked(m_scope == QLatin1String("sequence"));
    connect(sequence, &QAction::triggered, this, [this] { setScope(QStringLiteral("sequence")); });
    ui::popupMenu(menu, m_scopeButton);
}

void AskPrompt::open(const QPoint &anchor, const QString &title, const QString &clip) {
    m_title->setText(title);
    m_clip = clip;
    setScope(clip.isEmpty() ? QStringLiteral("sequence") : QStringLiteral("clip"));
    m_input->clear();
    adjustSize();
    // Above the anchor, kept on its screen.
    const QMargins shadow = ui::shadowMargins();
    QPoint at(anchor.x() - width() / 2, anchor.y() - height() + shadow.bottom() - 8);
    if (const QScreen *screen = QApplication::screenAt(anchor)) {
        const QRect area = screen->availableGeometry();
        at.setX(std::clamp(at.x(), area.left() + 8 - shadow.left(), area.right() - width() + shadow.right() - 8));
        if (at.y() + shadow.top() < area.top() + 8) at.setY(anchor.y() + 16 - shadow.top());
    }
    move(at);
    show();
    m_input->setFocus();
}

bool AskPrompt::eventFilter(QObject *object, QEvent *event) {
    if (object == m_input && event->type() == QEvent::KeyPress) {
        const auto *key = static_cast<QKeyEvent *>(event);
        if ((key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) && !(key->modifiers() & Qt::ShiftModifier)) {
            submit();
            return true;
        }
    }
    if (object == m_input && (event->type() == QEvent::FocusIn || event->type() == QEvent::FocusOut)) m_field->update();
    return QFrame::eventFilter(object, event);
}

void AskPrompt::submit() {
    const QString text = m_input->toPlainText().trimmed();
    if (text.isEmpty()) return;
    hide();
    emit submitted(text, scope());
}

} // namespace kerf
