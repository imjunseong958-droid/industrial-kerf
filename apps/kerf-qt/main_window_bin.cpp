// MainWindow's media bin, presets and the browser (mockup-spec 6 and 8).
// Q-BINBROWSER (W2); the region's own state and engine traffic are in
// ui/wiring/bin_wiring.*.

#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "stage_shell.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QAbstractTextDocumentLayout>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QClipboard>
#include <QColorDialog>
#include <QCursor>
#include <QDateTime>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QIcon>
#include <QImage>
#include <QJsonDocument>
#include <QKeySequence>
#include <QLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QScreen>
#include <QShortcut>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QHeaderView>
#include <QLabel>
#include <QTableWidget>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>


#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "editor_bus.h"
#include "wiring/bin_wiring.h"
#include "wiring/graph_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::exportEditingPreset() {
    QFileDialog dialog(this, QStringLiteral("Export editing preset"),
        QDir(m_projectDirectory).filePath(QStringLiteral("editing.kerfpreset")), QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    dialog.setAcceptMode(QFileDialog::AcceptSave);
    dialog.setDefaultSuffix(QStringLiteral("kerfpreset"));
    if (dialog.exec() != QDialog::Accepted) return;
    const QString path = dialog.selectedFiles().value(0);
    if (QFileInfo(path).suffix() != QStringLiteral("kerfpreset")) {
        showError(QStringLiteral("Save editing presets with the .kerfpreset extension."));
        return;
    }
    execute(QStringLiteral("file.exportPreset"), QJsonObject{{QStringLiteral("path"), path}});
}

void MainWindow::importEditingPreset() {
    const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import editing preset"),
        m_projectDirectory, QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    if (!path.isEmpty()) sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preset_inspect")},
                                               {QStringLiteral("path"), path}});
}

void MainWindow::showPresetBindings(const QString &path, const QJsonObject &preset) {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("presetBindingsDialog"));
    dialog.setWindowTitle(QStringLiteral("Apply preset — %1").arg(preset.value(QStringLiteral("name")).toString()));
    dialog.setMinimumWidth(560);
    auto *layout = new QVBoxLayout(&dialog);
    layout->setContentsMargins(20, 20, 20, 20);
    layout->setSpacing(12);
    auto *intro = new QLabel(QStringLiteral("Choose media for each slot. Kerf keeps the preset's timing, effects and animation. "
        "Applying replaces this sequence as a proposal for you to review."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *scroll = new QScrollArea;
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    auto *contents = new QWidget;
    auto *form = new QFormLayout(contents);
    form->setContentsMargins(0, 0, 0, 0);
    form->setVerticalSpacing(8);
    form->setHorizontalSpacing(12);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignTop);
    QHash<QString, QComboBox *> media;
    QHash<QString, QDoubleSpinBox *> starts;
    QHash<QString, QLineEdit *> texts;
    QHash<QString, QJsonObject> sources;
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        sources.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    // Kerf lists the media able to fill each slot and the latest frame each can
    // start from. Starts are shown in seconds but sent as whole frames.
    constexpr int latestFrameRole = Qt::UserRole + 1;
    constexpr int fpsRole = Qt::UserRole + 2;
    const QString root = preset.value(QStringLiteral("sequence")).toString();
    for (const QJsonValue &value : preset.value(QStringLiteral("slots")).toArray()) {
        const QJsonObject slot = value.toObject();
        const QString id = slot.value(QStringLiteral("id")).toString();
        auto *choice = new QComboBox;
        choice->setObjectName(QStringLiteral("presetMedia_%1").arg(id));
        choice->addItem(QStringLiteral("Choose imported media…"), QString());
        for (const QJsonValue &fitValue : slot.value(QStringLiteral("fits")).toArray()) {
            const QJsonObject fit = fitValue.toObject();
            const QJsonObject source = sources.value(fit.value(QStringLiteral("media")).toString());
            if (source.isEmpty()) continue;
            const QString sourceId = source.value(QStringLiteral("id")).toString();
            choice->addItem(QStringLiteral("%1 — %2").arg(sourceId,
                QFileInfo(source.value(QStringLiteral("path")).toString()).fileName()), sourceId);
            choice->setItemData(choice->count() - 1, fit.value(QStringLiteral("latestFrame")).toInteger(), latestFrameRole);
            choice->setItemData(choice->count() - 1, source.value(QStringLiteral("fps")).toDouble(), fpsRole);
        }
        if (choice->count() == 1) choice->setItemText(0, QStringLiteral("Import matching media first"));
        auto *from = new StepperSpinBox;
        from->setObjectName(QStringLiteral("presetFrom_%1").arg(id));
        // One width for every slot, so the steppers line up down the column.
        from->setFixedWidth(106);
        from->setDecimals(3);
        from->setSuffix(QStringLiteral(" s"));
        from->setRange(0, 0);
        from->setToolTip(QStringLiteral("Start in the replacement media, including the preset's transition handles"));
        connect(choice, &QComboBox::currentIndexChanged, from, [choice, from] {
            const double fps = choice->currentData(fpsRole).toDouble();
            from->setSingleStep(fps > 0 ? 1 / fps : 1);
            from->setMaximum(fps > 0 ? choice->currentData(latestFrameRole).toDouble() / fps : 0);
        });
        QStringList uses;
        for (const QJsonValue &clipValue : slot.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            QString use = QStringLiteral("%1 %2").arg(clip.value(QStringLiteral("track")).toString(),
                                                      clip.value(QStringLiteral("at")).toString());
            const QString sequence = clip.value(QStringLiteral("sequence")).toString();
            if (sequence != root) use = QStringLiteral("%1 › %2").arg(sequence, use);
            const QString name = clip.value(QStringLiteral("name")).toString();
            if (!name.isEmpty()) use += QStringLiteral(" “%1”").arg(name);
            uses << use;
        }
        auto *usedBy = new QLabel(uses.join(QStringLiteral(" · ")));
        usedBy->setObjectName(QStringLiteral("muted"));
        usedBy->setWordWrap(true);
        auto *row = new QWidget;
        auto *rowLayout = new QGridLayout(row);
        rowLayout->setContentsMargins(0, 0, 0, 0);
        rowLayout->setHorizontalSpacing(8);
        rowLayout->setVerticalSpacing(4);
        rowLayout->addWidget(choice, 0, 0);
        rowLayout->addWidget(from, 0, 1);
        rowLayout->addWidget(usedBy, 1, 0, 1, 2);
        // The media the preset makes of this slot's footage: what is chosen
        // here decides what they are made of.
        if (const QString readBy = presetReadByLine(presetSlotReadBy(preset, slot)); !readBy.isEmpty()) {
            auto *madeOf = new QLabel(readBy);
            madeOf->setObjectName(QStringLiteral("muted"));
            madeOf->setProperty("presetReadBy", id);
            madeOf->setWordWrap(true);
            rowLayout->addWidget(madeOf, 2, 0, 1, 2);
        }
        rowLayout->setColumnStretch(0, 1);
        rowLayout->setColumnStretch(1, 0);
        form->addRow(QStringLiteral("%1 · %2 s").arg(id).arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 3), row);
        media.insert(id, choice);
        starts.insert(id, from);
    }
    // The media the preset makes: shown, never bound. Applying adds them to
    // the project with their steps; Make… in the bin makes them.
    const QJsonArray made = preset.value(QStringLiteral("made")).toArray();
    if (!made.isEmpty()) {
        auto *makes = new QWidget;
        makes->setObjectName(QStringLiteral("presetMade"));
        auto *makesLayout = new QVBoxLayout(makes);
        makesLayout->setContentsMargins(0, 0, 0, 0);
        makesLayout->setSpacing(4);
        for (const QJsonValue &value : made) {
            const QJsonObject entry = value.toObject();
            const QString id = entry.value(QStringLiteral("id")).toString();
            auto *line = new QLabel(QStringLiteral("%1, %2").arg(id, presetMadeKind(entry)));
            line->setObjectName(QStringLiteral("presetMade_%1").arg(id));
            line->setWordWrap(true);
            QStringList steps;
            for (const QJsonValue &step : entry.value(QStringLiteral("steps")).toArray()) steps << step.toString();
            line->setToolTip(steps.join(QLatin1Char('\n')));
            makesLayout->addWidget(line);
        }
        auto *note = new QLabel(QStringLiteral("Made after applying, with Make… in the bin. They need no file."));
        note->setObjectName(QStringLiteral("muted"));
        note->setWordWrap(true);
        makesLayout->addWidget(note);
        form->addRow(QStringLiteral("Makes"), makes);
    }
    // Text slots by number, as the launcher shows them; their clip ids are
    // the document's names, in the tooltip.
    const QJsonObject textSlots = preset.value(QStringLiteral("textSlots")).toObject();
    int textNumber = 0;
    for (auto it = textSlots.begin(); it != textSlots.end(); ++it) {
        auto *text = new QLineEdit(it.value().toString());
        text->setObjectName(QStringLiteral("presetText_%1").arg(it.key()));
        text->setToolTip(it.key());
        form->addRow(QStringLiteral("Text %1").arg(++textNumber), text);
        texts.insert(it.key(), text);
    }
    scroll->setWidget(contents);
    layout->addWidget(scroll, 1);
    // The preset's source, for those who want it, behind a disclosure.
    auto *details = new QToolButton;
    details->setObjectName(QStringLiteral("disclosure"));
    details->setText(QStringLiteral("Details"));
    details->setCheckable(true);
    details->setAutoRaise(true);
    details->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    details->setIconSize(QSize(11, 11));
    details->setIcon(disclosureIcon(false, editorTheme().textSecondary));
    layout->addWidget(details, 0, Qt::AlignLeft);
    auto *code = new QPlainTextEdit(preset.value(QStringLiteral("document")).toString());
    code->setObjectName(QStringLiteral("presetCode"));
    code->setReadOnly(true);
    code->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    code->setMaximumHeight(150);
    code->hide();
    layout->addWidget(code);
    // As tall as what it shows, as macOS dialogs are: the slots up to most
    // of the screen (they scroll past that), and the source when Details
    // is open.
    const auto fit = [&dialog, layout] {
        layout->activate();
        dialog.resize(dialog.width(), layout->hasHeightForWidth() ? layout->totalHeightForWidth(dialog.width())
                                                                  : layout->totalSizeHint().height());
    };
    connect(details, &QToolButton::toggled, code, [details, code, fit](bool open) {
        code->setVisible(open);
        details->setIcon(disclosureIcon(open, editorTheme().textSecondary));
        fit();
    });
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Create Proposal"));
    buttons->button(QDialogButtonBox::Ok)->setDefault(true);
    const auto updateReady = [buttons, media] {
        bool ready = true;
        for (auto *choice : media) ready = ready && !choice->currentData().toString().isEmpty();
        buttons->button(QDialogButtonBox::Ok)->setEnabled(ready);
    };
    for (auto *choice : media) connect(choice, &QComboBox::currentIndexChanged, buttons, updateReady);
    updateReady();
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    dialog.ensurePolished();
    dialog.resize(600, dialog.height());
    const QMargins margins = layout->contentsMargins();
    const int inner = dialog.width() - margins.left() - margins.right();
    const int formHeight = form->hasHeightForWidth() ? form->totalHeightForWidth(inner) : form->totalSizeHint().height();
    const QScreen *screen = dialog.screen();
    scroll->setFixedHeight(std::min(formHeight, screen ? screen->availableGeometry().height() * 6 / 10 : 520));
    fit();
    const QJsonValue revision = revisionValue();
    if (dialog.exec() != QDialog::Accepted) return;
    if (revision != revisionValue()) {
        showError(QStringLiteral("The project changed while choosing preset media. Import the preset again."));
        return;
    }
    QJsonObject bindings;
    for (auto it = media.begin(); it != media.end(); ++it) {
        // Clamp: the shown maximum is rounded to the spin box's decimals.
        const qint64 frame = std::clamp<qint64>(
            qRound64(starts.value(it.key())->value() * it.value()->currentData(fpsRole).toDouble()),
            0, it.value()->currentData(latestFrameRole).toLongLong());
        bindings.insert(it.key(), QJsonObject{{QStringLiteral("media"), it.value()->currentData().toString()},
            {QStringLiteral("from"), QStringLiteral("%1f").arg(frame)}});
    }
    QJsonObject overrides;
    for (auto it = texts.begin(); it != texts.end(); ++it) {
        if (it.value()->isModified()) overrides.insert(it.key(), it.value()->text());
    }
    execute(QStringLiteral("file.applyPreset"), QJsonObject{{QStringLiteral("path"), path},
        {QStringLiteral("bindings"), bindings}, {QStringLiteral("texts"), overrides}});
}

void MainWindow::savePreset() {
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("Save Current as Preset"),
                                               QStringLiteral("Saves the program sequence in presets/, shared with the project.\nName"),
                                               QLineEdit::Normal, QString(), &ok)
                             .trimmed();
    if (!ok || name.isEmpty()) return;
    // The bridge names the file, cleaning the name up, and refuses one that
    // exists; replacing it is asked then.
    m_savingPreset = name;
    execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), false}});
}

void MainWindow::replaceSavedPreset(const QString &path) {
    const QString name = std::exchange(m_savingPreset, QString());
    if (QMessageBox::question(this, QStringLiteral("Save Current as Preset"),
                              QStringLiteral("%1 already exists. Replace it?").arg(path)) == QMessageBox::Yes)
        execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), true}});
}

QString MainWindow::mediaPath(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() == mediaId)
            return QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
    }
    return {};
}

void MainWindow::replaceMedia(const QString &mediaId, const QString &path) {
    QString file = path;
    if (file.isEmpty()) {
        file = QFileDialog::getOpenFileName(this, QStringLiteral("Replace %1").arg(mediaId), QFileInfo(mediaPath(mediaId)).absolutePath(),
                                            QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
        if (file.isEmpty()) return;
    }
    if (execute(QStringLiteral("media.replace"), QJsonObject{{QStringLiteral("media"), mediaId}, {QStringLiteral("path"), file}}))
        addActivity(QStringLiteral("import"), QStringLiteral("Replacing %1 with %2").arg(mediaId, QFileInfo(file).fileName()));
}

QString MainWindow::mediaName(const QString &mediaId) const {
    return sourceNames(m_project.value(QStringLiteral("sources")).toArray()).value(mediaId, mediaId);
}

QJsonObject MainWindow::mediaSource(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == mediaId) return value.toObject();
    return {};
}

QString MainWindow::mediaRecipe(const QString &mediaId) const {
    return mediaSource(mediaId).value(QStringLiteral("recipe")).toString();
}

bool MainWindow::mediaIsMade(const QString &mediaId) const { return sourceIsMade(mediaSource(mediaId)); }

QString MainWindow::mediaMadeFrom(const QString &mediaId) const { return sourceMadeFrom(mediaSource(mediaId)); }

QJsonArray MainWindow::mediaTakes(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() == mediaId) return source.value(QStringLiteral("takes")).toArray();
    }
    return {};
}

void MainWindow::useTake(const QString &mediaId, int take, const QString &path) {
    // A take is a file of the project already: the media's line names it, as a making's does.
    if (execute(QStringLiteral("media.replace"), QJsonObject{{QStringLiteral("media"), mediaId}, {QStringLiteral("path"), path}}))
        addActivity(QStringLiteral("import"), QStringLiteral("Using take %1 of %2").arg(take).arg(mediaId));
}

QStringList MainWindow::mediaShows(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() != mediaId) continue;
        QStringList shows;
        for (const QJsonValue name : source.value(QStringLiteral("shows")).toArray()) shows << name.toString();
        return shows;
    }
    return {};
}

void MainWindow::makeMedia(const QString &mediaId, int rolls) {
    QJsonObject params{{QStringLiteral("media"), mediaId}};
    if (rolls > 1) params.insert(QStringLiteral("rolls"), rolls);
    execute(QStringLiteral("media.make"), params);
}

void MainWindow::showCast() {
    sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("cast_get")}});
}

QString MainWindow::castSaved(const QJsonObject &cast) {
    QStringList wanting;
    for (const QJsonValue value : cast.value(QStringLiteral("toMake")).toArray())
        wanting << value.toObject().value(QStringLiteral("media")).toString();
    if (wanting.isEmpty()) return QStringLiteral("Saved the cast; no shot wants making for it");
    return QStringLiteral("Saved the cast; %1 %2 making again").arg(wanting.join(QStringLiteral(", ")),
                                                                   wanting.size() == 1 ? QStringLiteral("wants") : QStringLiteral("want"));
}

QDialog *MainWindow::castDialog(const QJsonObject &cast) {
    auto *dialog = new QDialog(this);
    dialog->setObjectName(QStringLiteral("castDialog"));
    dialog->setWindowTitle(QStringLiteral("Cast and Styles"));
    dialog->setMinimumSize(720, 420);
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    auto *layout = new QVBoxLayout(dialog);
    layout->setContentsMargins(20, 20, 20, 20);
    layout->setSpacing(12);
    auto *intro = new QLabel(QStringLiteral("Who and what this project's shots show, and how they look, said once. A recipe names them "
        "with cast= and style=, and a model is asked with their words and handed their pictures. Changing one asks again for the shots it is in."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *table = new QTableWidget(0, 4);
    table->setObjectName(QStringLiteral("castTable"));
    table->setHorizontalHeaderLabels({QStringLiteral("Kind"), QStringLiteral("Name"), QStringLiteral("Words"), QStringLiteral("Pictures")});
    table->horizontalHeader()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    table->horizontalHeader()->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    table->horizontalHeader()->setSectionResizeMode(2, QHeaderView::Stretch);
    table->horizontalHeader()->setSectionResizeMode(3, QHeaderView::Interactive);
    // Wide enough for two pictures' paths; the words take what is left.
    table->setColumnWidth(3, 280);
    table->horizontalHeader()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
    table->verticalHeader()->setVisible(false);
    table->setSelectionBehavior(QAbstractItemView::SelectRows);
    table->setSelectionMode(QAbstractItemView::SingleSelection);
    // A row: its kind, which is not typed, then what a person writes.
    const auto addRow = [table](const QString &kind, const QString &name, const QString &words, const QStringList &pictures) {
        const int row = table->rowCount();
        table->insertRow(row);
        auto *kindItem = new QTableWidgetItem(kind);
        kindItem->setFlags(kindItem->flags() & ~Qt::ItemIsEditable);
        table->setItem(row, 0, kindItem);
        table->setItem(row, 1, new QTableWidgetItem(name));
        table->setItem(row, 2, new QTableWidgetItem(words));
        auto *picturesItem = new QTableWidgetItem(pictures.join(QStringLiteral(", ")));
        picturesItem->setToolTip(QStringLiteral("Pictures of it, each a path from the project's folder, with a comma between them"));
        table->setItem(row, 3, picturesItem);
        return row;
    };
    for (const QJsonValue value : cast.value(QStringLiteral("entries")).toArray()) {
        const QJsonObject entry = value.toObject();
        QStringList pictures;
        for (const QJsonValue picture : entry.value(QStringLiteral("refs")).toArray()) pictures << picture.toString();
        addRow(entry.value(QStringLiteral("kind")).toString(), entry.value(QStringLiteral("name")).toString(),
               entry.value(QStringLiteral("words")).toString(), pictures);
    }
    layout->addWidget(table, 1);
    auto *row = new QHBoxLayout;
    row->setSpacing(8);
    const auto button = [row](const QString &text, const QString &name, const QString &tip) {
        auto *made = new QPushButton(text);
        made->setObjectName(name);
        made->setToolTip(tip);
        made->setAutoDefault(false);
        row->addWidget(made);
        return made;
    };
    auto *addCast = button(QStringLiteral("Add Cast"), QStringLiteral("castAdd"), QStringLiteral("Someone or something a shot shows: a person, a place, a prop"));
    auto *addStyle = button(QStringLiteral("Add Style"), QStringLiteral("castAddStyle"), QStringLiteral("How a shot looks: its stock, its grade, its light"));
    auto *addPicture = button(QStringLiteral("Add Picture…"), QStringLiteral("castPicture"),
                              QStringLiteral("A picture of the chosen row; one outside the project is copied into its refs folder"));
    auto *remove = button(QStringLiteral("Remove"), QStringLiteral("castRemove"), QStringLiteral("Take the chosen row out; a recipe that names it cannot run until it names another"));
    row->addStretch(1);
    layout->addLayout(row);
    const auto chosen = [table] { return table->currentRow(); };
    const auto follow = [table, addPicture, remove] {
        const bool has = table->currentRow() >= 0;
        addPicture->setEnabled(has);
        remove->setEnabled(has);
    };
    connect(table, &QTableWidget::currentCellChanged, dialog, follow);
    const auto added = [table, addRow](const QString &kind) {
        const int at = addRow(kind, QString(), QString(), {});
        table->setCurrentCell(at, 1);
        table->editItem(table->item(at, 1));
    };
    connect(addCast, &QPushButton::clicked, dialog, [added] { added(QStringLiteral("cast")); });
    connect(addStyle, &QPushButton::clicked, dialog, [added] { added(QStringLiteral("style")); });
    connect(remove, &QPushButton::clicked, dialog, [table, chosen, follow] {
        if (chosen() >= 0) table->removeRow(chosen());
        follow();
    });
    connect(addPicture, &QPushButton::clicked, dialog, [this, dialog, table, chosen] {
        if (chosen() < 0) return;
        const QString file = QFileDialog::getOpenFileName(dialog, QStringLiteral("Add Picture"), m_projectDirectory,
                                                          QStringLiteral("Pictures (*.png *.jpg *.jpeg *.webp *.tif *.tiff *.bmp)"));
        if (file.isEmpty()) return;
        const QString inside = castPicturePath(m_projectDirectory, file);
        if (inside.isEmpty()) {
            showError(QStringLiteral("Could not copy %1 into the project's refs folder").arg(QFileInfo(file).fileName()));
            return;
        }
        QTableWidgetItem *pictures = table->item(chosen(), 3);
        QStringList have = pictures->text().split(QLatin1Char(','), Qt::SkipEmptyParts);
        for (QString &one : have) one = one.trimmed();
        if (!have.contains(inside)) have << inside;
        pictures->setText(have.join(QStringLiteral(", ")));
    });
    follow();
    // What wants making as the cast stands, so a change is made knowing what it adds to.
    QStringList wanting;
    for (const QJsonValue value : cast.value(QStringLiteral("toMake")).toArray()) {
        const QJsonObject shot = value.toObject();
        wanting << QStringLiteral("%1 (%2)").arg(shot.value(QStringLiteral("media")).toString(), shot.value(QStringLiteral("why")).toString());
    }
    auto *wants = new QLabel(wanting.isEmpty() ? QStringLiteral("No shot wants making as the cast stands.")
                                               : QStringLiteral("Wants making now: %1").arg(wanting.join(QStringLiteral("; "))));
    wants->setObjectName(QStringLiteral("muted"));
    wants->setWordWrap(true);
    layout->addWidget(wants);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Save | QDialogButtonBox::Cancel);
    buttons->setObjectName(QStringLiteral("castButtons"));
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, dialog, &QDialog::reject);
    connect(buttons, &QDialogButtonBox::accepted, dialog, [this, dialog, table] {
        // A cell still being typed in is taken as it stands.
        table->setCurrentCell(-1, -1);
        QJsonArray entries;
        for (int at = 0; at < table->rowCount(); ++at) {
            QJsonArray pictures;
            for (const QString &picture : table->item(at, 3)->text().split(QLatin1Char(','), Qt::SkipEmptyParts))
                if (!picture.trimmed().isEmpty()) pictures.append(picture.trimmed());
            entries.append(QJsonObject{{QStringLiteral("kind"), table->item(at, 0)->text()},
                                       {QStringLiteral("name"), table->item(at, 1)->text().trimmed()},
                                       {QStringLiteral("words"), table->item(at, 2)->text().simplified()},
                                       {QStringLiteral("refs"), pictures}});
        }
        // The bridge reads it as the file would be read: what is wrong with a row is an error
        // that says so, and the file is as it was.
        if (sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("cast_set")}, {QStringLiteral("entries"), entries}})) dialog->accept();
    });
    return dialog;
}

QString MainWindow::castPicturePath(const QString &projectDirectory, const QString &file) {
    const QDir project(projectDirectory);
    const QString relative = project.relativeFilePath(QFileInfo(file).absoluteFilePath());
    // A picture of the project is named as it is; another is copied in, beside the others.
    if (!relative.startsWith(QStringLiteral("..")) && !QDir::isAbsolutePath(relative)) return relative;
    if (!project.mkpath(QStringLiteral("refs"))) return {};
    const QFileInfo info(file);
    QString name = info.fileName();
    for (int number = 2; QFileInfo::exists(project.filePath(QStringLiteral("refs/") + name)); ++number)
        name = QStringLiteral("%1-%2.%3").arg(info.completeBaseName()).arg(number).arg(info.suffix());
    return QFile::copy(file, project.filePath(QStringLiteral("refs/") + name)) ? QStringLiteral("refs/") + name : QString();
}

QString MainWindow::makeQuestion(const QJsonObject &plan) {
    // What its model has said it does not take, each worth mending before it is paid for.
    QString notes;
    for (const QJsonValue note : plan.value(QStringLiteral("notes")).toArray()) notes += QStringLiteral("\n• %1").arg(note.toString());
    if (!notes.isEmpty()) notes = QStringLiteral("\n\nBefore you run it:%1").arg(notes);
    // The shot goes by its recipe's name, as in the bin, not by its id; made by steps in the
    // document (an empty recipe), its id is its name.
    const QString recipe = plan.value(QStringLiteral("recipe")).toString();
    const QString media = recipe.isEmpty() ? plan.value(QStringLiteral("media")).toString() : recipeShotName(recipe);
    const QString from = recipe.isEmpty() ? QStringLiteral("its steps in the document") : QStringLiteral("the recipe %1").arg(recipe);
    // A recipe is commands someone wrote, perhaps someone else: show every step.
    const int rolls = plan.value(QStringLiteral("rolls")).toInt(1);
    if (rolls > 1) {
        const int take = plan.value(QStringLiteral("take")).toInt();
        return QStringLiteral("Making %1 runs this on your computer, %6 times:\n\n%2%5\n\nIt comes from %3 and writes %6 new takes, %7 to %8, "
                              "the first at %4, each with every seed one on from the last. %1 then is the last of them, and you choose "
                              "among them under Takes. The takes before them are kept.\n\nRun them?")
            .arg(media, plan.value(QStringLiteral("command")).toString(), from,
                 plan.value(QStringLiteral("file")).toString(), notes)
            .arg(rolls).arg(take).arg(take + rolls - 1);
    }
    return QStringLiteral("Making %1 runs this on your computer:\n\n%2%5\n\nIt comes from %3 and writes a new take, %4, which %1 then is. The takes before it are kept.\n\nRun it?")
        .arg(media, plan.value(QStringLiteral("command")).toString(), from, plan.value(QStringLiteral("file")).toString(), notes);
}

void MainWindow::runMake(const QString &mediaId, int rolls) {
    QJsonObject params{{QStringLiteral("media"), mediaId}, {QStringLiteral("confirmed"), true}};
    if (rolls > 1) params.insert(QStringLiteral("rolls"), rolls);
    if (execute(QStringLiteral("media.make"), params))
        addActivity(QStringLiteral("import"), rolls > 1 ? QStringLiteral("Making %3 takes of %1 from %2").arg(mediaId, mediaMadeFrom(mediaId)).arg(rolls)
                                                        : QStringLiteral("Making %1 from %2").arg(mediaId, mediaMadeFrom(mediaId)));
}

void MainWindow::stopMake(const QString &mediaId) {
    execute(QStringLiteral("media.makeCancel"), QJsonObject{{QStringLiteral("media"), mediaId}});
}

QString MainWindow::makingStatus(const QJsonObject &making, const QString &name, bool document) {
    const QString media = name.isEmpty() ? making.value(QStringLiteral("media")).toString() : name;
    const QString line = making.value(QStringLiteral("line")).toString();
    // Before any step has spoken, only that it began; then what the steps say.
    return line.isEmpty() ? QStringLiteral("Making take %2 of %1 from its %3…")
                                .arg(media)
                                .arg(making.value(QStringLiteral("take")).toInt())
                                .arg(document ? QStringLiteral("steps") : QStringLiteral("recipe"))
                          : QStringLiteral("Making %1: %2").arg(media, line);
}

void MainWindow::showGraph(const QString &mediaId) {
    if (!mediaIsMade(mediaId)) {
        showStatus(QStringLiteral("%1 is not made from a recipe or from steps, so there is no graph of how it is made").arg(mediaName(mediaId)), 4000);
        return;
    }
    m_graphWiring->open(mediaId);
}

void MainWindow::openGraph(const QString &media, const QString &node) { m_graphWiring->openWhenReady(media, node); }

QString MainWindow::clipMedia(const QString &clipId) const {
    return clipObject(clipId).value(QStringLiteral("sourceId")).toString();
}

void MainWindow::revealMedia(const QString &mediaId) {
    const QString path = mediaPath(mediaId);
    if (path.isEmpty()) return;
#if defined(Q_OS_MACOS)
    QProcess::startDetached(QStringLiteral("open"), {QStringLiteral("-R"), path});
#else
    QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(path).absolutePath()));
#endif
}

QJsonObject MainWindow::binMedia() const {
    const QString id = m_projectPanel ? m_projectPanel->currentMedia() : QString();
    if (id.isEmpty()) return {};
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == id) return value.toObject();
    return {};
}

QStringList MainWindow::sourceTargets() const {
    // The timeline's targeted track of each kind the bin's media has (its
    // heads' target toggles; by default the first unlocked of each kind).
    // A kind whose target is switched off is not edited.
    const QJsonObject source = binMedia();
    const QStringList targeted = m_timeline ? m_timeline->targetTracks() : QStringList{};
    QStringList targets;
    for (const QString &kind : {QStringLiteral("video"), QStringLiteral("audio")}) {
        const bool has = kind == QStringLiteral("video") ? source.contains(QStringLiteral("width"))
                                                         : source.value(QStringLiteral("hasAudio")).toBool();
        if (!has) continue;
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            const QString id = track.value(QStringLiteral("id")).toString();
            if (track.value(QStringLiteral("type")).toString() == kind && !track.value(QStringLiteral("locked")).toBool() &&
                targeted.contains(id)) {
                targets.append(id);
                break;
            }
        }
    }
    return targets;
}

QMenu *MainWindow::binMenu(const QString &mediaId) {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("binMenu"));
    menu->setToolTipsVisible(true);
    if (!mediaId.isEmpty()) {
        // The menu is for the media under the pointer, which is the one chosen.
        menu->addAction(m_commands->action(QStringLiteral("source.insert")));
        menu->addAction(m_commands->action(QStringLiteral("source.overwrite")));
        menu->addSeparator();
        QAction *replace = menu->addAction(QStringLiteral("Replace Media…"));
        replace->setObjectName(QStringLiteral("binReplace"));
        replace->setEnabled(m_commands->isEnabled(QStringLiteral("media.replace")));
        replace->setToolTip(replace->isEnabled() ? QStringLiteral("Swap the file behind %1 for everyone; its clips keep their timing").arg(mediaId)
                                                 : m_commands->reason(QStringLiteral("media.replace")));
        connect(replace, &QAction::triggered, this, [this, mediaId] { replaceMedia(mediaId); });
        // Made from a recipe file, or by steps written under its line in the document.
        const QString recipe = mediaRecipe(mediaId);
        const bool made = mediaIsMade(mediaId);
        const bool document = made && recipe.isEmpty();
        if (made) {
            // How it is made, to see and to set: its steps and what they read, wired.
            QAction *graph = menu->addAction(QStringLiteral("Open Graph"));
            graph->setObjectName(QStringLiteral("binGraph"));
            graph->setToolTip(document ? QStringLiteral("See how %1 is made and set it: its steps in the document and what they read, as a graph").arg(mediaId)
                                       : QStringLiteral("See how %1 is made and set it: the steps of %2 and what they read, as a graph").arg(mediaId, recipe));
            connect(graph, &QAction::triggered, this, [this, mediaId] { showGraph(mediaId); });
        }
        if (m_making.contains(mediaId)) {
            // While it is being made, the menu stops it and does not start another.
            QAction *stop = menu->addAction(QStringLiteral("Stop Making"));
            stop->setObjectName(QStringLiteral("binMakeStop"));
            stop->setEnabled(m_commands->isEnabled(QStringLiteral("media.makeCancel")));
            stop->setToolTip(stop->isEnabled() ? QStringLiteral("Stop making %1: its step is asked to end, no take is kept, and %1 stays as it is").arg(mediaId)
                                               : m_commands->reason(QStringLiteral("media.makeCancel")));
            connect(stop, &QAction::triggered, this, [this, mediaId] { stopMake(mediaId); });
        } else if (made) {
            QAction *make = menu->addAction(document ? QStringLiteral("Make…") : QStringLiteral("Make from Recipe…"));
            make->setObjectName(QStringLiteral("binMake"));
            make->setEnabled(m_commands->isEnabled(QStringLiteral("media.make")));
            // Who and what it shows, where its steps name any of the project's cast.
            const QStringList shows = mediaShows(mediaId);
            const QString showing = shows.isEmpty() ? QString() : QStringLiteral("; it shows %1").arg(shows.join(QStringLiteral(", ")));
            make->setToolTip(make->isEnabled() ? QStringLiteral("Make %1's file again by running %2; the command is shown first%3").arg(mediaId, mediaMadeFrom(mediaId), showing)
                                               : m_commands->reason(QStringLiteral("media.make")));
            connect(make, &QAction::triggered, this, [this, mediaId] { makeMedia(mediaId); });
            // Several takes of the same shot to choose among, each with its seeds one on.
            auto *several = new QMenu(document ? QStringLiteral("Make Several") : QStringLiteral("Make Several from Recipe"), menu);
            several->setObjectName(QStringLiteral("binMakeSeveral"));
            several->setToolTipsVisible(true);
            several->setEnabled(make->isEnabled());
            for (const int rolls : {2, 3, 4}) {
                QAction *roll = several->addAction(QStringLiteral("%1 Takes…").arg(rolls));
                roll->setObjectName(QStringLiteral("binMake%1").arg(rolls));
                roll->setToolTip(QStringLiteral("Make %1 takes of %2, each with every seed of its %3 one on from the last; the command is shown first")
                                     .arg(rolls)
                                     .arg(mediaId, document ? QStringLiteral("steps") : QStringLiteral("recipe")));
                connect(roll, &QAction::triggered, this, [this, mediaId, rolls] { makeMedia(mediaId, rolls); });
            }
            menu->addMenu(several);
        }
        if (const QJsonArray takes = mediaTakes(mediaId); !takes.isEmpty()) {
            // Every take is kept: the one the media is is ticked, and another is chosen here.
            auto *chosen = new QMenu(QStringLiteral("Takes"), menu);
            chosen->setObjectName(QStringLiteral("binTakes"));
            chosen->setToolTipsVisible(true);
            const bool can = m_commands->isEnabled(QStringLiteral("media.replace")) && !m_making.contains(mediaId);
            for (const QJsonValue &value : takes) {
                const QJsonObject take = value.toObject();
                const int number = take.value(QStringLiteral("number")).toInt();
                const QString path = take.value(QStringLiteral("path")).toString();
                const bool is = take.value(QStringLiteral("chosen")).toBool();
                QAction *use = chosen->addAction(QStringLiteral("Take %1").arg(number));
                use->setObjectName(QStringLiteral("binTake%1").arg(number));
                use->setCheckable(true);
                use->setChecked(is);
                use->setEnabled(can && !is);
                use->setToolTip(is ? QStringLiteral("%1 is this take").arg(mediaId)
                                : can ? QStringLiteral("Have %1 be %2 again; its clips keep their timing, and it is undone as any edit").arg(mediaId, path)
                                      : m_making.contains(mediaId) ? QStringLiteral("%1 is being made").arg(mediaId)
                                                                   : m_commands->reason(QStringLiteral("media.replace")));
                connect(use, &QAction::triggered, this, [this, mediaId, number, path] { useTake(mediaId, number, path); });
            }
            menu->addMenu(chosen);
        }
        QAction *reveal = menu->addAction(QStringLiteral("Reveal in Finder"));
        reveal->setObjectName(QStringLiteral("binReveal"));
        reveal->setEnabled(QFileInfo::exists(mediaPath(mediaId)));
        connect(reveal, &QAction::triggered, this, [this, mediaId] { revealMedia(mediaId); });
        menu->addSeparator();
    }
    menu->addAction(m_commands->action(QStringLiteral("file.import")));
    menu->addAction(m_commands->action(QStringLiteral("collab.consolidate")));
    QAction *cast = menu->addAction(QStringLiteral("Cast and Styles…"));
    cast->setObjectName(QStringLiteral("binCast"));
    cast->setToolTip(QStringLiteral("Who and what this project's shots show, and how they look, said once for every recipe"));
    connect(cast, &QAction::triggered, this, &MainWindow::showCast);
    return menu;
}

// The media bin.
void MainWindow::buildBin() {
    m_projectPanel = new ProjectPanel(m_artwork);
    // Double-click puts the media in at the playhead, as Insert does; what
    // Insert and Overwrite can do follows the media chosen.
    connect(m_projectPanel, &ProjectPanel::openRequested, this, [this](const QString &) {
        updateCommandStates();
        if (m_commands->isEnabled(QStringLiteral("source.insert"))) triggerCommand(QStringLiteral("source.insert"));
        else showStatus(m_commands->reason(QStringLiteral("source.insert")), 4000);
    });
    connect(m_projectPanel, &ProjectPanel::currentMediaChanged, this, &MainWindow::updateCommandStates);
    connect(m_projectPanel, &ProjectPanel::menuRequested, this, [this](const QString &media, const QPoint &at) {
        QMenu *menu = binMenu(media);
        menu->exec(at);
        menu->deleteLater();
    });
    // The bin's Index and Roles, and the region's engine traffic.
    m_binWiring->build();
}

} // namespace kerf
